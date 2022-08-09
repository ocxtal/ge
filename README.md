
# ge − grep and edit git-tracked files in bulk

**ge** is a tool to edit grep match locations all at once in a single editor pane. It allows us to make the most of the features of modern editors like multi-cursor editing and arbitrary undo-and-redoes, without losing the flexibility and handiness of the command-line grep utilities. It is especially powerful if the target files have properties like:

* each edit location consists of multiple lines
* and these lines are not exactly the same

Such situations are very common when maintaining a large codebase; for example:

* reordering or extending arguments of a function that is used all around the project
  * caller sides sometimes have different linebreak positions, indentation levels, and/or names for the arguments
* modifying configuration files that are almost the same but different in details
  * in recent days we have to maintain CI/CD-related files for { x86_64, aarch64 } x { Linux, Windows, macOS }
* renaming a field of a struct defined in some serialization schema
  * the struct is used in different languages like JavaScript and Go

![example](./figs/example.png)

*Figure 1. Editing two `split_whitespace`s in different files at once, which are found with the keyword `split`.*

ge performs the following four steps when invoked:

* queries the input word (can be in a regular expression) with **git grep**
* composes a **"half diff"**
* **launches an editor** for users to edit the half diff, and then waits for the user
* converts the edited half diff to a regular unified diff, and feeds it to **git apply**

## Using different editors

You can use any editor that can be launched from the terminal.

* **vim**: `--editor=vim`
  * You may need to add `:set backupcopy=yes` to your `.vimrc` to prevent ge from losing tempfiles. See [here](http://vimdoc.sourceforge.net/htmldoc/options.html#'backupcopy') for the details.
* **VSCode**: `--editor="code --wait --reuse-window"`
  * Needs the **[Command Line Interface](https://code.visualstudio.com/docs/editor/command-line)** set up in your environment.
  * Needs the `--wait` option for the `code` to wait for the user.
  * `--reuse-window` is recommended if you are in a terminal in VSCode, as it prevents the `code` from opening another window.
* **Sublime Text**: `--editor="rsubl --wait"`
  * Needs the **[Remote Subl](https://github.com/randy3k/RemoteSubl)** plugin installed.
  * Needs the `--wait` option for the `rsubl` to wait for the user.

ge recognizes the environment variable `EDITOR` as well. Note that the `--editor` option takes precedence over the environment variable.

## Arguments and options

```bash
ge [--editor=EDITOR] [GREP RANGE OPTIONS] PATTERN
```

It has one mandatory positional argument:

* `PATTERN` to search with **git grep**. Can be a regular expression (See the `--mode` option for the details).

Some options control the range to extract with grep:

* `--after-context=N` adds N lines after matches
* `--before-context=N` adds N lines before matches
* `--context=N` adds N lines before and after matches
* `--funciton-context` extends every match to the entire function
* `--to=PATTERN` extends matches downward until the first hit of `PATTERN`

And some options to control the output:

* `--editor=EDITOR` overrides the editor to use. The default is `vi`.
* `--preview` only dumps half diffs if specified.
* `--pager=PAGER` overrides the drain for the `--preview` mode. The default is `less -F`.

It has some more options such as `--header=HEADER` and `--hunk=HUNK`. See `ge --help` for the details of these extra options and the shorthand form of the basic options above.

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

* The `+++` starting at the head of a line is a "header marker", followed by a space and a filename without escaping. It indicates the series of hunks below the header is from the file.
* The `@@` starting at the head of a line is a "hunk marker", followed by a location the hunk took place in the `linenumber,linecount` format. The series of lines below the hunk marker constitutes one grep hit context.
* No line marker, `+` nor `-`, is appended at the head of each line as we don't need to distinguish the original and target lines.
  * Half diffs contain only the target lines.
  * If the target lines contain a string that collides with the header or hunk marker, please use the `--header` or `--hunk` option to change the markers.

## Installation

[Rust toolchain](https://rustup.rs/) is required.

```bash
cargo install --git https://github.com/ocxtal/ge
```

or

```bash
git clone https://github.com/ocxtal/ge.git
cd ge
cargo build --release
# `ge` is built in `./target/release`. copy it anywhere you want.
```

## Notes

* It doesn't support editing files not tracked by git. It's my design decision to use git as safety equipment to prevent irreparable destruction.
* Not tested on Windows. I don't think it works as it depends on possibly-unix-only features.

## Copyright and license

Hajime Suzuki (2022). Licensed under MIT.
