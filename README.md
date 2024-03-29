
# ge − grep and edit git-tracked files in bulk

**ge** is a tool to edit grep match locations all at once in a single editor pane. It allows us to make the most of the features of modern editors, like multi-cursor editing and arbitrary undo-and-redoes, without losing the flexibility and handiness of the command-line grep utilities.

ge is especially powerful when we edit almost identical but slightly different code fragments spread across multiple files. Such situations are quite common when maintaining a large codebase; for example:

* Reordering or extending arguments of a function that is used all around the project.
  * Caller sides sometimes have different linebreak positions, indentation levels, and/or names for the arguments.
* Modifying configuration files that are almost the same but different in detail.
  * In recent days we have to maintain CI/CD-related files for { x86_64, aarch64 } x { Linux, Windows, macOS }.
* Renaming a field of a struct defined in some serialization schema.
  * The struct is used in different languages like JavaScript and Go.

![example](./figs/example.png)

*Figure 1. Editing two `split_whitespace`s in different files at once, which are found with the keyword `split`.*

## How it works

ge performs the following four steps when invoked:

* Queries the input word (`PATTERN`; can be in a regular expression) with **git grep**
* Composes a **"half diff"**
* **Launches an editor** for users to edit the half diff and then waits for the user
* Converts the edited half diff to a regular unified diff and feeds it to **git apply**

## Usage

```console
$ ge --preview "pattern-of-interest"
```

* `--preview` (or `-p` in short) searches "pattern-of-interest" in your codebase and print hit locations; it works almost the same as command-line grep utilities.

```console
$ ge "pattern-of-interest"
```

* Without `--preview`, it will launch an editor with hit locations. After editing some lines, saving the contents, and exiting the editor, you'll find the codes are updated with the edits you made.
  * See the section ["Half diffs explained"](#half-diffs-explained) for the structure of contents loaded to the editor.

### Using different editors

You can use any editor that can be launched from the terminal.

* **vim**: `--editor=vim`
  * You may need to add `:set backupcopy=yes` to your `.vimrc` to prevent ge from losing tempfiles. See [here](http://vimdoc.sourceforge.net/htmldoc/options.html#'backupcopy') for the details.
* **VSCode**: `--editor="code --wait --reuse-window"`
  * It needs the **[Command Line Interface](https://code.visualstudio.com/docs/editor/command-line)** set up in your environment.
  * It needs the `--wait` option for the `code` to wait for the user.
  * `--reuse-window` is recommended if you are in a terminal in VSCode, as it prevents the `code` from opening another window.
* **Sublime Text**: `--editor="rsubl --wait"`
  * It needs the **[Remote Subl](https://github.com/randy3k/RemoteSubl)** plugin installed.
  * It needs the `--wait` option for the `rsubl` to wait for the user.

ge recognizes the environment variable `EDITOR` as well. Note that the `--editor` option takes precedence over the environment variable.

### Complete list of arguments

```console
$ ge --help
ge 0.0.2
grep and edit git-tracked files in bulk

USAGE:
    ge [OPTIONS] <PATTERN>

ARGS:
    <PATTERN>    Pattern to search

OPTIONS:
    -A, --after-context <N>     Include <N> additional lines after matches
    -B, --before-context <N>    Include <N> additional lines before matches
    -C, --context <N>           Include <N> additional lines before and after matches
    -e, --editor <EDITOR>       Use <EDITOR> to edit matches [default: vi]
    -h, --help                  Print help information
    -H, --head <N>              Edit <N> lines from the head of files that have matches
        --header <MARKER>       Use <MARKER> for header markers [default: +++]
        --hunk <MARKER>         Use <MARKER> for hunk markers [default: @@]
    -i, --ignore-case           Case-insensitive search
    -M, --mode <MODE>           Regex mode [default: basic] [possible values: fixed, extended,
                                basic, pcre]
        --max-depth <N>         Maximum directory depth to search [default: inf]
    -p, --preview               Show matches and exit
        --pager <PAGER>         Use <PAGER> to preview matches [default: less -F]
        --to <PATTERN>          Extend match downward until the first hit of PATTERN
    -V, --version               Print version information
    -w, --word-regexp           Match at word boundaries
    -W, --function-context      Extend match to the entire function
        --with <PATTERN>        Filter out files that don't have the PATTERN
        --without <PATTERN>     Filter out files that have the PATTERN
    -x, --exclude <PATHSPEC>    Files to exclude in search (in pathspec; multiple allowed)
    -y, --only <PATHSPEC>       Files to search (in pathspec; multiple allowed)
```

## "Half diffs" explained

Half diff is a unified diff format with only the target lines. The original lines are cached inside ge during editing and don't appear in the file edited by the user. A typical half diff looks like this:

```rust
+++ src/editor.rs
@@ 95,1
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
@@ 101,1
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
+++ src/git.rs
@@ 33,1
    context: Option<usize>,
@@ 41,1
    before: Option<usize>,
@@ 49,1
    after: Option<usize>,
@@ 71,1
    max_depth: Option<usize>,
```

* The `+++` starting at the head of a line is a "header marker," followed by a space and a filename without escaping. It indicates the series of hunks below the header is from the file.
* The `@@` starting at the head of a line is a "hunk marker," followed by a location the hunk took place in the `linenumber,linecount` format. The series of lines below the hunk marker constitutes one grep hit context.
* No line marker, `+` nor `-`, is appended at the head of each line as we don't need to distinguish the original and target lines.
  * Half diffs contain only the target lines.
  * If the target lines contain a string that collides with the header or hunk marker, please use the `--header` or `--hunk` option to change the markers.

## Installation

### Prebuilt binaries

```console
$ curl -OL https://github.com/ocxtal/ge/releases/download/0.0.2/ge-stable-x86_64-unknown-linux-musl.tar.gz
$ tar xf ge-stable-x86_64-unknown-linux-musl.tar.gz
  # `ge` is expanded at the current directory. Copy it anywhere you want. 
$ ./ge --help
```

* [Linux x86\_64 (static binary)](https://github.com/ocxtal/ge/releases/download/0.0.2/ge-stable-x86_64-unknown-linux-musl.tar.gz)
* [Linux aarch64 (static binary)](https://github.com/ocxtal/ge/releases/download/0.0.2/ge-stable-aarch64-unknown-linux-musl.tar.gz)
* [macOS x86\_64](https://github.com/ocxtal/ge/releases/download/0.0.2/ge-stable-x86_64-apple-darwin.tar.gz)

### From source

[Rust toolchain](https://rustup.rs/) is required.

```console
$ cargo install ge --git https://github.com/ocxtal/ge
$ ge --help
```

or

```console
$ git clone https://github.com/ocxtal/ge.git
$ cd ge
$ cargo build --release
  # `ge` is built in `./target/release`. Copy it anywhere you want.
$ ./target/release/ge --help
```

## Notes

* It doesn't support editing files not tracked by git. It's my design decision to use git as safety equipment to prevent irreparable destruction.
* It is not tested on Windows. I don't think it works, as it depends on possibly-unix-only features.

## Copyright and license

Hajime Suzuki (2022). Licensed under MIT.
