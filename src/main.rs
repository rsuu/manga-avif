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
    let mut args = Args::new();
    args.start();
}

pub struct Item {
    pub path: String,
    pub data: Vec<u8>,
}

impl Args {
    fn start(&mut self) {
        if self.archive_ty == ArchiveType::Unknown {
            unimplemented!()
        }

        let path = Path::new(self.path.as_str());

        // TODO:
        // from: zip file
        if let Some(ext) = path.extension() {
            let ext = ext.to_str().unwrap();

            if ext.ends_with(".zip") || ext.ends_with(".cbz") {
                self.from_zip();
            }

        // from: pipe
        } else if path == Path::new("-") {
            self.from_pipe();
        } else {
            self.from_dir();
        }
        // TODO: from: http
    }

    fn from_dir(&mut self) {
        let path = self.path.as_str();
        self.list = get_filelist(path.as_ref());
        let fpath = path.to_string();
        let dir = self.dir.as_str();

        dbg!(&self.list);

        let dst = match self.archive_ty {
            ArchiveType::Zip => format!("{dir}/{fpath}.cbz"),
            _ => unimplemented!(),
        };

        //
        if Path::new(dst.as_str()).is_file() {
            println!("SKIP: {dst}");

            std::process::exit(0);
        }

        //

        self.dst = dst;

        if self.dst.ends_with(".cbz") {
            self.to_cbz();
        }
    }

    fn from_zip(&self) {}
    fn from_pipe(&self) {
        let mut buffer = Vec::new();
        let stdin = io::stdin();
        let mut handle = stdin.lock();

        handle.read_to_end(&mut buffer).unwrap();

        let mut res = vec![];

        if infer::is(&buffer, "zip") {
            println!("zip");

            let mut zip = ZipArchive::new(Cursor::new(&buffer)).unwrap();
            let len = zip.len();

            for idx in 0..len {
                let mut f = zip.by_index(idx).unwrap();
                let fname = f.name().to_string();

                let mut data = Vec::new();
                f.read_to_end(&mut data).unwrap();

                res.push((fname, data));
            }

            dbg!(zip.len());
        } else {
            dbg!(buffer.len());
        }

        dbg!(res.len());

        todo!()
    }

    fn to_cbz(&mut self) {
        let Args {
            path,
            speed,
            quality,
            depth,
            list,
            ..
        } = self;
        println!("START: {path}");

        let vec = Arc::new(Mutex::new(Vec::with_capacity(list.len())));

        let f = |p: &PathBuf| {
            let vec = vec.clone();
            let mut vec = vec.lock().unwrap();

            //dbg!(&p);
            if let Ok(data) = img2avif(p.as_path(), *speed, *quality, *depth) {
                // rename to avif
                let mut img_path = p.to_path_buf();
                img_path.set_extension(AVIF_EXT);

                println!("DONE: {}", img_path.display());

                vec.push(Item {
                    path: img_path.display().to_string(),
                    data,
                })
            } else {
                vec.push(Item {
                    path: p.display().to_string(),
                    data: vec![],
                })
            }
        };
        for v in list.chunks(2) {
            v.par_iter().map(f).collect::<()>();
        }

        let vec = vec.clone();
        let mut vec = vec.lock().unwrap();

        'a: for Item { data, .. } in vec.iter() {
            if data.is_empty() {
                self.archive_ty = ArchiveType::Dir;
                break 'a;
            }
        }

        dbg!(vec.len());
        match self.archive_ty {
            ArchiveType::Zip => self.inner_to_cbz(&mut vec),

            ArchiveType::Dir => self.inner_to_dir(&mut vec),

            _ => {}
        }
    }

    fn inner_to_dir(&self, vec: &mut [Item]) {
        let Args { dir, path, .. } = self;

        let dir_path = format!("./{dir}/{path}");
        let dir_path = Path::new(&dir);

        // WARN:
        if dir_path.is_file() {
            return;
        }

        // if exitis { do nothing }
        let _ = fs::create_dir(dir_path);
        println!("mkdir: {}", dir_path.display());

        for Item { path, data } in vec.iter_mut() {
            if data.is_empty() {
                println!("WARN<missed>: {path}");
                continue;
            }

            let dst = format!("{dir}/{path}");
            dbg!(&dst);

            let mut f = fs::File::create(dst).unwrap();
            f.write_all(data.as_slice()).unwrap();
        }

        println!("DONE<DIR>: {path}");
    }

    fn inner_to_cbz(&self, vec: &mut [Item]) {
        let Args {
            put_to,
            addr,
            port,

            dst,
            ..
        } = self;

        let mut ar = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut ar);

            let options = FileOptions::default().compression_method(zip::CompressionMethod::Stored);

            for Item { path, data } in vec.iter_mut() {
                println!("{}", data.len());

                // zip
                zip.start_file(path.as_str(), options).unwrap();
                zip.write_all(data.as_slice()).unwrap();

                data.clear();
                data.shrink_to(0);
            }
            zip.finish().unwrap();
        }

        ar.seek(io::SeekFrom::Start(0)).unwrap();

        // net
        if let Some(path) = put_to {
            let url = format!("http://{addr}:{port}/{path}/{dst}");
            curl_put(ar, &url);

            println!("DONE<curl_put>: {url}");
        // disk
        } else {
            let data = ar.get_ref().as_slice();
            fs::write(dst, data).unwrap();

            println!("DONE<cbz>: {dst}");
        }
    }
}

struct Args {
    path: String,
    put_to: Option<String>,
    log_file: Option<String>,

    dir: String,

    speed: u8,
    quality: u8,
    depth: u8,
    archive_ty: ArchiveType,

    addr: String,
    port: u16,

    dst: String,
    list: Vec<PathBuf>,
}

#[derive(PartialEq)]
enum ArchiveType {
    Zip,

    Pipe,

    Dir,

    Unknown,
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

fn img2avif(path: &Path, speed: u8, quality: u8, depth: u8) -> Result<Vec<u8>, ()> {
    let encode = ravif::Encoder::new()
        .with_num_threads(Some(8))
        .with_depth(Some(depth))
        .with_quality(quality as f32)
        .with_speed(speed);

    println!("start: {}", path.display());

    // `?` vs `let else`
    let Ok(img) = ImageReader::open(path) else {
        return Err(());
    };
    let Ok(img) = img.decode() else {
        return Err(());
    };

    let width = img.width() as usize;
    let height = img.height() as usize;

    dbg!(img.color());

    // NOTE: No as_rgba8(), because we MUST convert image color to rgba
    let buf = img.into_rgba8();
    let buf = buf.as_rgba();

    dbg!(buf.len());
    let res = encode
        .encode_rgba(ravif::Img::new(buf, width, height))
        .unwrap()
        .avif_file
        .to_vec();
    dbg!(res.len());

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

        // flag
        let put_to = args.opt_value_from_str("--put-to").unwrap();

        // img
        let speed = args.value_from_str("--speed").unwrap_or(4);
        let quality = args.value_from_str("--quality").unwrap_or(70);
        let depth = args.value_from_str("--depth").unwrap_or(10);
        let archive_ty = args
            .value_from_str("--ext")
            .map(|f: String| ArchiveType::from(f.as_str()))
            .unwrap();

        // http
        let addr = args
            .value_from_str("--addr")
            .unwrap_or("localhost".to_string());
        let port = args.value_from_str("--port").unwrap_or(5000);

        //
        let log_file = args.opt_value_from_str("--log").unwrap();

        // move to
        let dir = args.value_from_str("--dir").unwrap_or(".".to_string());

        // file
        let path = args
            .free_from_str::<String>()
            .expect("Path argument is required");

        let _ = args.finish();

        Self {
            path,
            speed,
            quality,
            dir,
            log_file,
            depth,
            put_to,
            archive_ty,
            addr,
            port,

            dst: "".to_string(),
            list: vec![],
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
