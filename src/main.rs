use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::collections::HashMap;
use std::env::var;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::process::{Command, Stdio};
use tempfile::NamedTempFile;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    pattern: String,

    #[clap(short, long)]
    context: Option<usize>,

    #[clap(short, long)]
    before: Option<usize>,

    #[clap(short, long)]
    after: Option<usize>,

    #[clap(short, long)]
    preview: bool,

    #[clap(long, default_value = "+++")]
    header: String,

    #[clap(long, default_value = "@@")]
    hunk: String,

    #[clap(short, long)]
    editor: Option<String>,
}

struct Git;

struct GrepArgs<'a> {
    pattern: &'a str,
    context: Option<usize>,
    before: Option<usize>,
    after: Option<usize>,
}

impl Git {
    fn new() -> Result<Self> {
        // check the availability of the git command
        let output = Command::new("git")
            .args(&["--version"])
            .output()
            .context("\"git\" command not found.")?;
        assert!(output.status.success());

        Ok(Git)
    }

    fn grep(&self, args: &GrepArgs) -> Result<String> {
        // compose arguments
        let mut grep_args = vec![
            "grep".to_string(),
            "--color=never".to_string(),
            "--line-number".to_string(),
        ];
        if let Some(c) = args.context {
            grep_args.push(format!("--context={}", c));
        }
        if let Some(b) = args.before {
            grep_args.push(format!("--before={}", b));
        }
        if let Some(a) = args.after {
            grep_args.push(format!("--after={}", a));
        }
        grep_args.push(args.pattern.to_string());

        // run git-grep then parse the output as a utf-8 string
        let output = Command::new("git")
            .args(&grep_args)
            .output()
            .context("failed to get output of \"git grep\". aborting...")?;
        let output = String::from_utf8(output.stdout).context(
            "failed to interpret the output of \"git grep\" as a UTF-8 string. aborting...",
        )?;

        Ok(output)
    }

    fn apply(&self, patch: &str) -> Result<()> {
        let mut apply = Command::new("git")
            .args(&["apply", "--allow-empty", "--unidiff-zero", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("failed to run \"git apply\". aborting...")?;

        // we expect it's dropped after use (it sends EOF)
        {
            let mut stdin = apply.stdin.take().unwrap();
            stdin.write_all(patch.as_bytes()).unwrap();
        }

        // make sure patch was successful
        let code = apply
            .wait()
            .context("\"git apply\" unexpectedly exited. aborting...")?;
        if !code.success() {
            return Err(anyhow!(
                "\"git apply\" returned an error{}. aborting...",
                code
            ));
        }

        Ok(())
    }
}

struct HalfDiffConfig<'a> {
    header: &'a str,
    hunk: &'a str,
}

struct PatchBuilder<'a> {
    config: &'a HalfDiffConfig<'a>,
    files: HashMap<String, usize>,
    lines: HashMap<(usize, usize), (usize, String)>,
}

impl<'a> PatchBuilder<'a> {
    fn from_grep(config: &'a HalfDiffConfig, raw: &str) -> Result<Self> {
        let mut locs = PatchBuilder {
            config,
            files: HashMap::new(),
            lines: HashMap::new(),
        };

        locs.parse_grep(raw)?;
        Ok(locs)
    }

    fn parse_grep(&mut self, raw: &str) -> Result<()> {
        let mut prev_id = 0;
        let mut prev_pos = 0;
        let mut prev_base_pos = 0;
        for l in raw.trim().lines() {
            let v: Vec<_> = l.splitn(3, &[':', '-'][..]).collect();
            if v.len() != 3 {
                return Err(anyhow!("unexpected grep line: {}. aborting...", l));
            }

            if v[0] == "" {
                debug_assert!(v[1] == "" && v[2] == "");
                (prev_id, prev_pos, prev_base_pos) = (0, 0, 0);
                continue;
            }

            let filename = &v[0];
            let ln: usize = v[1]
                .parse()
                .with_context(|| format!("broken grep line number: {}. aborting...", &v[1]))?;
            debug_assert!(ln > 0);

            let next_id = self.files.len() + 1;
            let id = *self.files.entry(filename.to_string()).or_insert(next_id);
            debug_assert!(id > 0);

            if prev_id == id && prev_pos == ln - 1 {
                // continues
                self.lines.entry((id, prev_base_pos)).and_modify(|e| {
                    e.0 = ln + 1 - prev_base_pos;
                    e.1.push_str(&v[2]); // we may need to add a prefix here
                    e.1.push('\n');
                });
            } else {
                self.lines.insert((id, ln), (1, format!("{}\n", v[2])));
                prev_base_pos = ln;
            }

            prev_id = id;
            prev_pos = ln;
        }

        Ok(())
    }

    fn write_halfdiff(&self, drain: &mut dyn Write) -> Result<()> {
        // index files
        let index: HashMap<usize, &str> = self.files.iter().map(|x| (*x.1, x.0.as_str())).collect();

        // format and dump file content
        let mut keys: Vec<_> = self.lines.keys().collect();
        keys.sort();

        let mut prev_id = 0;
        for &(id, pos) in keys {
            if prev_id != id {
                let filename = index.get(&id).unwrap();
                drain.write_all(format!("{} {}\n", self.config.header, filename).as_bytes())?;
                prev_id = id;
            }

            let (len, content) = self.lines.get(&(id, pos)).unwrap();
            drain.write_all(
                format!("{} {},{}\n{}", self.config.hunk, pos, len, content).as_bytes(),
            )?;
        }

        Ok(())
    }

    fn read_halfdiff(&self, src: &mut dyn Read) -> Result<String> {
        let mut buf = Vec::new();
        src.read_to_end(&mut buf)
            .context("failed to read the edit result. aborting...")?;

        let mut patch = String::new();
        let mut acc = HunkAccumulator::new(&self.lines);

        let diff = std::str::from_utf8(&buf)
            .context("failed parse the edit result as a UTF-8 string. aborting...")?;

        for l in diff.lines() {
            if l.starts_with(&self.config.header) {
                acc.dump_hunk(&mut patch);

                let filename = l[self.config.header.len()..].trim();
                patch.push_str(&format!("--- a/{}\n+++ b/{}\n", filename, filename));

                let filename = self.files.get(filename).unwrap();
                acc.open_new_file(*filename);
            } else if l.starts_with(&self.config.hunk) {
                acc.dump_hunk(&mut patch);
                acc.open_new_hunk(l[self.config.hunk.len()..].trim());
            } else {
                acc.push_line(l);
            }
        }
        acc.dump_hunk(&mut patch);

        Ok(patch)
    }
}

struct Editor {
    args: Vec<String>,
    file: NamedTempFile,
}

impl Editor {
    fn new(editor: &str) -> Result<Self> {
        // create tempfile first
        let file = NamedTempFile::new().context("failed to create tempfile. aborting...")?;
        let name = file.path().to_str().unwrap().to_string();

        // break it by spaces to extract the base command
        let mut args: Vec<_> = editor.split_whitespace().map(|x| x.to_string()).collect();

        // check if it exists
        if !Self::exists(&args[0]) {
            return Err(anyhow!(
                "failed to find the editor {:?} in the PATH. aborting...",
                &args[0]
            ));
        }

        // if it's a vim, disable inode swapping
        if Self::is_a_vim(&args[0]) {
            args.push("-c".to_string());
            args.push(":set backupcopy=yes".to_string());
        }

        // add the target file
        args.push(name);

        Ok(Editor { args, file })
    }

    fn exists(editor: &str) -> bool {
        let output = Command::new("which").args(&[editor]).output();
        if output.is_err() {
            return false;
        }

        output.unwrap().status.success()
    }

    fn is_a_vim(editor: &str) -> bool {
        let output = Command::new(editor).args(&["--version"]).output();
        if output.is_err() {
            return false;
        }

        let output = output.unwrap();
        if !output.status.success() {
            // it doesn't support "--version" flag. apparently not a vim
            return false;
        }

        &output.stdout[..3] == b"VIM".as_slice()
    }

    fn wait_edit(&mut self) -> Result<()> {
        // invoke the actual process here
        let mut editor = Command::new(&self.args[0])
            .args(&self.args[1..])
            .spawn()
            .with_context(|| {
                format!("failed to start the editor: {}. aborting...", self.args[0])
            })?;
        let output = editor
            .wait()
            .context("editor exited unexpectedly. aborting...")?;
        if !output.success() {
            return Err(anyhow!("editor exited and returned an error. aborting..."));
        }

        // make sure the tempfile exists
        // (vim and some other editors creates another working file and rename it to the original on quit,
        // which cause a missing-tempfile error. so here we check the tempfile we created still exists
        // with the same inode, by re-opening the file after the editor finished.)
        let _file = self.file.reopen().context(
            "the tempfile is missing (the editor might have closed or changed the inode of the file). aborting...",
        )?;

        // seek to the head before reading the content...
        self.file
            .seek(SeekFrom::Start(0))
            .context("failed to seek the tempfile. aborting...")?;

        Ok(())
    }
}

impl Read for Editor {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl Write for Editor {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    // create git and editor objects
    let git = Git::new()?;
    let mut editor = Editor::new(
        args.editor
            .as_deref()
            .unwrap_or(var("EDITOR").as_deref().unwrap_or("vi")),
    )?;

    // run git-grep collect hits
    let grep_output = git.grep(&GrepArgs {
        pattern: &args.pattern,
        context: args.context,
        before: args.before,
        after: args.after,
    })?;

    // parse the result
    let config = &HalfDiffConfig {
        header: &args.header,
        hunk: &args.hunk,
    };
    let gen = PatchBuilder::from_grep(&config, &grep_output)?;

    // convert the git-grep result (hit locations) into "halfdiff" that will be edited by the user
    {
        let mut writer = BufWriter::new(&mut editor);
        gen.write_halfdiff(&mut writer)?;
        writer
            .flush()
            .context("failed flush the tempfile. aborting...")?;
    }

    // wait for the user...
    editor.wait_edit()?;

    // read the edit result, and parse it into a unified diff
    let mut reader = BufReader::new(&mut editor);
    let patch = gen.read_halfdiff(&mut reader)?;

    // then apply the patch
    git.apply(&patch)?;

    // we've done all
    Ok(())
}

struct HunkAccumulator<'a, 'b> {
    id: usize,
    hunk: &'a str,
    buf: String,
    edited_len: usize,
    pos_diff: isize,
    original: &'b HashMap<(usize, usize), (usize, String)>,
}

impl<'a, 'b> HunkAccumulator<'a, 'b> {
    fn new(original: &'b HashMap<(usize, usize), (usize, String)>) -> Self {
        HunkAccumulator {
            id: 0,
            hunk: "",
            buf: String::new(),
            edited_len: 0,
            pos_diff: 0,
            original,
        }
    }

    fn is_empty(&self) -> bool {
        self.id == 0 || self.hunk == ""
    }

    fn open_new_file(&mut self, id: usize) {
        self.id = id;
    }

    fn open_new_hunk(&mut self, hunk: &'a str) {
        self.hunk = hunk;
        self.buf.clear();
        self.edited_len = 0;
    }

    fn push_line(&mut self, line: &str) {
        self.buf.push_str(line);
        self.buf.push('\n');
        self.edited_len += 1;
    }

    fn dump_hunk(&mut self, patch: &mut String) {
        if self.is_empty() {
            self.open_new_hunk("");
            return;
        }

        let hunk: Vec<_> = self.hunk.split(',').collect();
        let original_pos = hunk[0].parse().unwrap();
        let (original_len, content) = self.original.get(&(self.id, original_pos)).unwrap();
        let original_len = *original_len;

        if &self.buf == content {
            self.open_new_hunk("");
            return;
        }

        patch.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            original_pos,
            original_len,
            (original_pos as isize + self.pos_diff) as usize,
            self.edited_len
        ));
        for l in content.lines() {
            patch.push('-');
            patch.push_str(l);
            patch.push('\n');
        }
        for l in self.buf.lines() {
            patch.push('+');
            patch.push_str(l);
            patch.push('\n');
        }

        self.pos_diff += self.edited_len as isize;
        self.pos_diff -= original_len as isize;
        self.open_new_hunk("");
    }
}
