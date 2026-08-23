import os
import sys
import shutil
import subprocess
import re


def copy_executable(src: str, dest: str) -> None:
    """Copy an executable and make sure it is executable."""
    os.makedirs(os.path.dirname(dest), exist_ok=True)

    shutil.copy2(src, dest)
    os.chmod(dest, 0o755)

    print(f"[SUCCESS] Copied: {src} -> {dest}")


def get_ldd_dependencies(binary: str) -> set[str]:
    """
    Return all shared-library dependencies reported by ldd.

    This only returns the direct dependencies of the binary.
    Recursive dependencies are resolved by calling this function
    on every discovered library.
    """
    try:
        result = subprocess.run(
            ["ldd", binary],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )
    except FileNotFoundError:
        print("[PANIC] 'ldd' is not available on this machine!")
        sys.exit(1)

    if result.returncode != 0:
        print(f"[PANIC] Failed to inspect dependencies for: {binary}")
        print(result.stderr)
        sys.exit(1)

    dependencies = set()

    for line in result.stdout.splitlines():
        line = line.strip()

        # Typical output:
        #
        # libavcodec.so.61 => /usr/lib/x86_64-linux-gnu/libavcodec.so.61 (...)
        #
        match = re.search(r"=>\s*(/[^\s]+)", line)

        if match:
            path = match.group(1)
            if os.path.isfile(path):
                dependencies.add(os.path.realpath(path))
            continue

        # Also handles:
        #
        # /lib64/ld-linux-x86-64.so.2 (...)
        #
        # linux-vdso.so.1 is ignored because it is not a real file.
        match = re.match(r"(/[^\s]+)\s+\(", line)

        if match:
            path = match.group(1)
            if os.path.isfile(path):
                dependencies.add(os.path.realpath(path))

    return dependencies


def collect_all_dependencies(binary: str) -> set[str]:
    """
    Recursively collect every shared library required by a binary.
    """
    collected = set()
    pending = [os.path.realpath(binary)]

    while pending:
        current = pending.pop()

        if current in collected:
            continue

        if not os.path.isfile(current):
            continue

        collected.add(current)

        for dependency in get_ldd_dependencies(current):
            if dependency not in collected:
                pending.append(dependency)

    return collected


def copy_library_preserving_links(src: str, destination_dir: str) -> None:
    """
    Copy a library into destination_dir.

    If src is a symlink, preserve its link structure and also copy
    the real target into the same directory.
    """
    os.makedirs(destination_dir, exist_ok=True)

    filename = os.path.basename(src)
    destination = os.path.join(destination_dir, filename)

    # Resolve the actual library file.
    real_src = os.path.realpath(src)

    # Copy the actual file first.
    real_filename = os.path.basename(real_src)
    real_destination = os.path.join(destination_dir, real_filename)

    if not os.path.exists(real_destination):
        shutil.copy2(real_src, real_destination)
        print(f"[LIB] {real_src} -> {real_destination}")

    # Re-create the symlink if the original was a symlink.
    if os.path.islink(src):
        link_target = os.readlink(src)

        # Remove an old destination if present.
        if os.path.lexists(destination):
            os.remove(destination)

        os.symlink(
            os.path.basename(link_target),
            destination,
        )

        print(f"[LINK] {destination} -> {link_target}")

    elif src != real_src:
        # This shouldn't normally happen, but protects against unusual
        # filesystem layouts.
        if not os.path.exists(destination):
            shutil.copy2(src, destination)


def install_external_dependencies():
    """
    Package external runtime dependencies into:

        libs/
        ├── yt-dlp/
        │   └── yt-dlp
        └── ffmpeg/
            ├── ffmpeg
            ├── ffprobe
            └── all required shared libraries
    """

    script_dir = os.path.dirname(os.path.abspath(__file__))
    project_root = os.path.dirname(script_dir)
    libs_dir = os.path.join(project_root, "libs")

    # ytdlp_dir = os.path.join(libs_dir, "yt-dlp")
    ffmpeg_dir = os.path.join(libs_dir, "ffmpeg")

    # os.makedirs(ytdlp_dir, exist_ok=True)
    os.makedirs(ffmpeg_dir, exist_ok=True)

    print(f"[INFO] Project root: {project_root}")
    print(f"[INFO] Libraries directory: {libs_dir}")

    # =========================================================
    # STEP 1: yt-dlp
    # =========================================================

    print("\n[STEP 1] Locating yt-dlp...")

    ytdlp_path = shutil.which("yt-dlp")

    if not ytdlp_path:
        print("[PANIC] yt-dlp is not installed on this machine!")
        sys.exit(1)

    try:
        dest_ytdlp = os.path.join(libs_dir, "yt-dlp")
        copy_executable(ytdlp_path, dest_ytdlp)

    except Exception as e:
        print(f"[PANIC] Failed to package yt-dlp: {e}")
        sys.exit(1)

    # =========================================================
    # STEP 2: ffmpeg
    # =========================================================

    print("\n[STEP 2] Locating ffmpeg...")

    ffmpeg_path = shutil.which("ffmpeg")

    if not ffmpeg_path:
        print("[PANIC] ffmpeg is not installed on this machine!")
        sys.exit(1)

    try:
        dest_ffmpeg = os.path.join(ffmpeg_dir, "ffmpeg")
        copy_executable(ffmpeg_path, dest_ffmpeg)

    except Exception as e:
        print(f"[PANIC] Failed to package ffmpeg: {e}")
        sys.exit(1)

    # =========================================================
    # STEP 3: ffprobe
    # =========================================================

    print("\n[STEP 3] Locating ffprobe...")

    ffprobe_path = shutil.which("ffprobe")

    if not ffprobe_path:
        print(
            "[PANIC] ffprobe is not installed on this machine! "
            "(Usually comes with ffmpeg)"
        )
        sys.exit(1)

    try:
        dest_ffprobe = os.path.join(ffmpeg_dir, "ffprobe")
        copy_executable(ffprobe_path, dest_ffprobe)

    except Exception as e:
        print(f"[PANIC] Failed to package ffprobe: {e}")
        sys.exit(1)

    # =========================================================
    # STEP 4: Collect ALL ffmpeg/ffprobe runtime libraries
    # =========================================================

    print("\n[STEP 4] Collecting ffmpeg runtime libraries...")

    try:
        ffmpeg_dependencies = collect_all_dependencies(ffmpeg_path)
        ffprobe_dependencies = collect_all_dependencies(ffprobe_path)

        all_dependencies = ffmpeg_dependencies | ffprobe_dependencies

        # Do not copy the executables themselves as libraries.
        all_dependencies.discard(os.path.realpath(ffmpeg_path))
        all_dependencies.discard(os.path.realpath(ffprobe_path))

        print(
            f"[INFO] Found {len(all_dependencies)} "
            f"runtime libraries."
        )

        for library in sorted(all_dependencies):
            copy_library_preserving_links(
                library,
                ffmpeg_dir,
            )

    except Exception as e:
        print(
            f"[PANIC] Failed to collect ffmpeg libraries: {e}"
        )
        sys.exit(1)

    # =========================================================
    # DONE
    # =========================================================

    print("\n[DONE] External dependencies packaged successfully.")


def main():
    install_external_dependencies()


if __name__ == "__main__":
    main()