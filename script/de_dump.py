import sys
import os
from collections import Counter


class Args:
    l = sys.argv[1]
    r = sys.argv[2]
    pass


# fd -t d -d 1
def fd_dir(path, depth=1):
    for root, dirs, files in os.walk(path):
        current_depth = root[len(path) + len(os.path.sep) :].count(os.path.sep)
        if current_depth <= depth:
            if current_depth == depth:
                print(root)
            else:
                if dirs:
                    print(root)
                else:
                    print(root + " (empty)")


path = "."
depth = 1

fd_dir(path, depth)


def fd_dir():
    for item in os.listdir("."):
        if not item.startswith("."):
            print(item)


def main():
    f = sys.argv[1]

    with open(f, "r") as file:
        # rstrip(): remove \n
        lines = [clean_end(line.rstrip()) for line in file.readlines()]

        l = []
        for elem, count in Counter(lines).items():
            if count == 1:
                l.append(elem)
            else:
                pass

        print("\n".join(l))


def clean_end(filename):
    if filename.endswith("/"):
        filename = filename[:-1]

    else:
        filename = os.path.splitext(filename)[0]

    return filename


main()
