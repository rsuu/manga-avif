use image::{io::Reader as ImageReader, DynamicImage};
use infer;
use pico_args::Arguments;
use ravif;
use rayon::prelude::*;
use rgb::FromSlice;
use std::{
    fs::{self, OpenOptions},
    io::{self, prelude::*, Cursor, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{atomic::AtomicBool, Arc, Mutex},
};
use walkdir::WalkDir;
use zip::{write::FileOptions, ZipArchive, ZipWriter};

const AVIF_EXT: &str = "avif";

fn main() {
    let args = Args::new();
    args.start();
}

pub struct Args {
    src: PathBuf,
    dst: PathBuf,

    src_ty: ArchiveType,
    dst_ty: ArchiveType,

    speed: u8,
    quality: u8,
    depth: u8,

    flag_force: bool,
    num_threads: usize,
}

#[derive(PartialEq)]
pub enum ArchiveType {
    // `filename.{zip, cbz}`
    Zip,

    // `-`
    Pipe,

    // `path/`
    Dir,

    Unknown,
}

pub struct Item {
    pub path: PathBuf,
    pub data: DataTy,
}

pub enum DataTy {
    File(Vec<u8>),
    Dir,
}

impl Args {
    fn start(&self) {
        if self.src_ty == ArchiveType::Unknown || self.dst_ty == ArchiveType::Unknown {
            unimplemented!()
        }

        match self.src_ty {
            ArchiveType::Dir => self.from_dir(),
            //ArchiveType::Pipe => self.from_pipe(),
            //ArchiveType::Zip => self.from_zip(),
            _ => todo!(),
        }
    }

    fn from_dir(&self) {
        //dbg!("from_dir()");

        match self.dst_ty {
            ArchiveType::Zip => {
                if self.dst.exists() {
                    if self.flag_force {
                        fs::remove_file(self.dst.as_path()).unwrap();
                    } else {
                        eprintln!("Error: exitis {}", self.dst.display());

                        std::process::exit(-1);
                    }
                }

                self.to_zip();
            }
            _ => unimplemented!(),
        }
    }

    //    fn from_zip(&self) {}
    //    fn from_pipe(&self) {
    //        let mut buffer = Vec::new();
    //        let stdin = io::stdin();
    //        let mut handle = stdin.lock();
    //
    //        handle.read_to_end(&mut buffer).unwrap();
    //
    //        let mut res = vec![];
    //
    //        if infer::is(&buffer, "zip") {
    //            println!("zip");
    //
    //            let mut zip = ZipArchive::new(Cursor::new(&buffer)).unwrap();
    //            let len = zip.len();
    //
    //            for idx in 0..len {
    //                let mut f = zip.by_index(idx).unwrap();
    //                let fname = f.name().to_string();
    //
    //                let mut data = Vec::new();
    //                f.read_to_end(&mut data).unwrap();
    //
    //                res.push((fname, data));
    //            }
    //
    //            dbg!(zip.len());
    //        } else {
    //            dbg!(buffer.len());
    //        }
    //
    //        dbg!(res.len());
    //
    //        todo!()
    //    }

    fn to_zip(&self) {
        //dbg!("to_zip()");

        let Args {
            speed,
            quality,
            depth,
            ..
        } = self;
        let list = get_filelist(self.src.as_ref());
        let vec = Arc::new(Mutex::new(Vec::with_capacity(list.len())));

        let f = |p: &PathBuf| {
            let vec = vec.clone();
            let mut vec = vec.lock().unwrap();

            if p.is_file() {
                let data = img2avif(p.as_path(), *speed, *quality, *depth).unwrap();

                // rename to avif
                let mut path = p.to_path_buf();
                path.set_extension(AVIF_EXT);

                vec.push(Item {
                    path,
                    data: DataTy::File(data),
                });
            } else if p.is_dir() {
                vec.push(Item {
                    path: p.clone(),
                    data: DataTy::Dir,
                });
            } else {
                unimplemented!()
            }

            println!("\tDONE: {}", p.display());
        };
        for v in list.chunks(self.num_threads) {
            v.par_iter().map(f).collect::<()>();
        }

        let vec = vec.clone();
        let mut vec = vec.lock().unwrap();

        self.inner_to_zip(&mut vec);
    }

    //    fn inner_to_dir(&self, vec: &mut [Item]) {
    //        let Args { dir, path, .. } = self;
    //
    //        let dir_path = format!("./{dir}/{path}");
    //        let dir_path = Path::new(&dir);
    //
    //        // WARN:
    //        if dir_path.is_file() {
    //            return;
    //        }
    //
    //        // if exitis { do nothing }
    //        let _ = fs::create_dir(dir_path);
    //        println!("mkdir: {}", dir_path.display());
    //
    //        for Item { path, data } in vec.iter_mut() {
    //            if data.is_empty() {
    //                println!("WARN<missed>: {path}");
    //                continue;
    //            }
    //
    //            let dst = format!("{dir}/{path}");
    //            dbg!(&dst);
    //
    //            let mut f = fs::File::create(dst).unwrap();
    //            f.write_all(data.as_slice()).unwrap();
    //        }
    //
    //        println!("DONE<DIR>: {path}");
    //    }

    fn inner_to_zip(&self, vec: &mut [Item]) {
        let Args { dst, .. } = self;

        let mut ar = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut ar);

            let options = FileOptions::default().compression_method(zip::CompressionMethod::Stored);

            for Item { path, data } in vec.iter_mut() {
                match data {
                    DataTy::File(v) => {
                        zip.start_file(path.display().to_string(), options).unwrap();
                        zip.write_all(v.as_slice()).unwrap();

                        v.clear();
                        v.shrink_to(0);
                    }

                    DataTy::Dir => {
                        zip.add_directory(path.display().to_string(), options)
                            .unwrap();
                    }
                }
            }

            zip.finish().unwrap();
        }

        ar.seek(io::SeekFrom::Start(0)).unwrap();

        let data = ar.get_ref().as_slice();
        fs::write(dst, data).unwrap();

        println!("DONE<cbz>: {}", dst.display());
    }
}

fn img2avif(path: &Path, speed: u8, quality: u8, depth: u8) -> Result<Vec<u8>, ()> {
    let encode = ravif::Encoder::new()
        .with_num_threads(Some(8))
        .with_depth(Some(depth))
        .with_quality(quality as f32)
        .with_speed(speed);

    //dbg!("start: {}", path.display());

    // `?` vs `let else`
    let img = ImageReader::open(path).unwrap().decode().unwrap();
    //dbg!(img.color());

    let width = img.width() as usize;
    let height = img.height() as usize;

    // NOTE: No as_rgba8(), because we MUST convert image color to rgba
    let buf = img.into_rgba8();
    let buf = buf.as_rgba();
    //dbg!(buf.len());

    let res = encode
        .encode_rgba(ravif::Img::new(buf, width, height))
        .unwrap()
        .avif_file
        .to_vec();
    //dbg!(res.len());

    Ok(res)
}

fn curl_put(ar: Cursor<Vec<u8>>, url: &str) {
    let mut child = Command::new("sh")
        .arg("-c")
        .stdin(Stdio::piped())
        .arg({
            // src: pipe
            let cmd = "curl -T -";

            format!("{cmd} {url}",)
        })
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().expect("Failed to open stdin");

    std::thread::spawn(move || {
        let data = ar.get_ref().as_slice();
        stdin.write_all(data).expect("Failed to write to stdin");
    });

    child.wait().expect("Failed to read stdout");

    println!("DONE: curl_put");
}

impl Args {
    fn new() -> Self {
        let mut args = Arguments::from_env();

        if args.contains("--help") {
            print_help();
        }
        // file
        let src: PathBuf = args.value_from_str("--src").unwrap();
        let dst: PathBuf = args.value_from_str("--dst").unwrap();
        let src_ty = {
            if let Some(ext) = src.extension() {
                ArchiveType::from(ext.to_str().unwrap())
            } else if src.starts_with("http://") || src.starts_with("https://") {
                // http
                let addr = args
                    .value_from_str("--addr")
                    .unwrap_or("localhost".to_string());
                let port = args.value_from_str("--port").unwrap_or(5000);

                todo!()
            } else {
                ArchiveType::Dir
            }
        };
        let dst_ty = {
            if let Some(ext) = dst.extension() {
                ArchiveType::from(ext.to_str().unwrap())
            } else if src.starts_with("http://") || src.starts_with("https://") {
                // http
                let addr = args
                    .value_from_str("--addr")
                    .unwrap_or("localhost".to_string());
                let port = args.value_from_str("--port").unwrap_or(5000);

                todo!()
            } else {
                ArchiveType::Dir
            }
        };

        // opt

        // img
        let speed = args.value_from_str("--speed").unwrap_or(4);
        let quality = args.value_from_str("--quality").unwrap_or(70);
        let depth = args.value_from_str("--depth").unwrap_or(10);

        // misc
        let flag_force = args.contains("--force");
        let num_threads = args.value_from_str("--threads").unwrap_or(8);

        let _ = args.finish();

        Self {
            src,
            dst,

            src_ty,
            dst_ty,

            speed,
            quality,
            depth,

            flag_force,
            num_threads,
        }
    }
}

impl From<&Path> for ArchiveType {
    fn from(value: &Path) -> Self {
        if value
            .extension()
            .is_some_and(|ext| ext == "zip" || ext == "cbz")
        {
            Self::Zip

        // from: pipe
        } else if value == Path::new("-") {
            Self::Pipe
        } else {
            Self::Unknown
        }
    }
}

impl From<&str> for ArchiveType {
    fn from(value: &str) -> Self {
        match value {
            "cbz" | "zip" => Self::Zip,
            "-" => Self::Pipe,

            _ => Self::Unknown,
        }
    }
}

impl From<&ArchiveType> for String {
    fn from(value: &ArchiveType) -> Self {
        match value {
            ArchiveType::Zip => "cbz",

            _ => unreachable!(),
        }
        .to_string()
    }
}

fn print_help() {
    println!(
        "
--speed
--quality
--depth
--ext
--dir

manga-avif
  --speed 4 --quality 70 --depth 10
  --ext cbz --dir ./
  tests
"
    );
    std::process::exit(0);
}

fn get_filelist(path: &Path) -> Vec<PathBuf> {
    let mut res = Vec::new();
    for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        if entry
            .path()
            .extension()
            .is_some_and(|e| e == "jpg" || e == "jpeg" || e == "png" || e == "webp")
        {
            res.push(entry.path().to_path_buf());
        }
    }

    res.sort();
    res
}
