# Install leyline

Use a published binary if you want to chat with leyline. Building from a
checkout is for contributors and people who deliberately want to build from
source.

## Download a release binary

Open the project's [GitHub Releases](https://github.com/jhanekom27/leyline/releases)
page and download the archive for your platform:

- Apple Silicon macOS: `leyline-aarch64-apple-darwin.tar.xz`
- Intel macOS: `leyline-x86_64-apple-darwin.tar.xz`
- x86_64 Linux: `leyline-x86_64-unknown-linux-gnu.tar.xz`
- x86_64 Windows: `leyline-x86_64-pc-windows-msvc.zip`

Every archive has a matching `.sha256` file. Download both, then verify the
archive before extracting it:

```sh
shasum -a 256 -c leyline-aarch64-apple-darwin.tar.xz.sha256
```

On macOS or Linux, extract the archive and move `leyline` to a directory on
your `PATH`:

```sh
tar -xf leyline-aarch64-apple-darwin.tar.xz
install -m 755 leyline-aarch64-apple-darwin/leyline ~/.local/bin/leyline
```

On Windows, use Explorer's **Extract All** action for the ZIP archive, then
move `leyline.exe` to a directory on your `PATH`. PowerShell can verify the
published checksum with:

```powershell
(Get-FileHash leyline-x86_64-pc-windows-msvc.zip -Algorithm SHA256).Hash
```

Compare that value to the hash in the matching `.sha256` file.

Run `leyline --version` to confirm that the installed executable is available.

## Install from source with Cargo

If you already use Rust, Cargo can build leyline from a specific published
release tag:

```sh
cargo install --git https://github.com/jhanekom27/leyline --tag v<VERSION> leyline
```

Replace `<VERSION>` with a release version such as `0.18.0`. This path needs a
supported Rust toolchain and compilation tools; use a prebuilt archive when
you only want to run leyline.

## Contribute to leyline

For development, clone the repository and build from the checkout:

```sh
git clone https://github.com/jhanekom27/leyline.git
cd leyline
cargo run
```

Contributors use the repository's Rust and Taskfile commands to build, test,
and maintain the project. That tooling is not needed to install a published
binary.
