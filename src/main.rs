use clap::Parser;
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
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

fn main() {
    let args = Args::parse();

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
    grep_args.push(args.pattern.clone());

    // grep the pattern with git-grep
    let grep = Command::new("git").args(&grep_args).output();
    if grep.is_err() {
        panic!("failed to get output of \"git grep\"");
    }

    // save original lines and compose file content to edit
    let mut filenames: HashMap<String, usize> = HashMap::new();
    let mut original: HashMap<(usize, usize), (usize, String)> = HashMap::new();

    let grep_output = &grep.unwrap().stdout;
    let grep_output = std::str::from_utf8(grep_output).unwrap();

    let mut prev_id = 0;
    let mut prev_pos = 0;
    let mut prev_base_pos = 0;
    for l in grep_output.trim().lines() {
        let v: Vec<_> = l.splitn(3, &[':', '-'][..]).collect();

        if v[0] == "" {
            debug_assert!(v[1] == "" && v[2] == "");
            (prev_id, prev_pos, prev_base_pos) = (0, 0, 0);
            continue;
        }

        let filename = &v[0];
        let ln: usize = v[1].parse().unwrap();
        debug_assert!(ln > 0);

        let next_id = filenames.len() + 1;
        let id = *filenames.entry(filename.to_string()).or_insert(next_id);
        debug_assert!(id > 0);

        if prev_id == id && prev_pos == ln - 1 {
            // continues
            original.entry((id, prev_base_pos)).and_modify(|e| {
                e.0 = ln + 1 - prev_base_pos;
                e.1.push_str(&v[2]); // we may need to add a prefix here
                e.1.push('\n');
            });
        } else {
            original.insert((id, ln), (1, format!("{}\n", v[2])));
            prev_base_pos = ln;
        }

        prev_id = id;
        prev_pos = ln;
    }

    // index files
    let index: HashMap<usize, &str> = filenames.iter().map(|x| (*x.1, x.0.as_str())).collect();

    // format and dump file content
    let mut keys: Vec<_> = original.keys().collect();
    keys.sort();

    let mut file = BufWriter::new(NamedTempFile::new().unwrap());

    let mut prev_id = 0;
    for &(id, pos) in keys {
        if prev_id != id {
            let filename = index.get(&id).unwrap();
            file.write_all(format!("{} {}\n", args.header, filename).as_bytes())
                .unwrap();
            prev_id = id;
        }

        let (len, content) = original.get(&(id, pos)).unwrap();
        file.write_all(format!("{} {},{}\n{}", args.hunk, pos, len, content).as_bytes())
            .unwrap();
    }
    file.flush().unwrap();

    // edit!
    let file = file.into_inner().unwrap();
    let name = file.path().to_str().unwrap().to_string();

    let editor = if let Some(editor) = args.editor {
        editor
    } else {
        std::env::var("EDITOR").unwrap_or("vi".to_string())
    };
    let mut editor: Vec<_> = editor.split_whitespace().collect();
    editor.push(&name);

    let mut editor = Command::new(editor[0]).args(&editor[1..]).spawn().unwrap();
    editor.wait().unwrap();

    // reload the content
    let mut file = BufReader::new(file.reopen().unwrap());
    let mut v = Vec::new();
    file.read_to_end(&mut v).unwrap();

    // parse
    let mut patch = Vec::new();
    let mut acc = HunkAccumulator::new(&original);
    for l in std::str::from_utf8(&v).unwrap().lines() {
        if l.starts_with(&args.header) {
            acc.dump_hunk(&mut patch);

            let filename = l[args.header.len()..].trim();
            patch.extend_from_slice(format!("--- a/{}\n+++ b/{}\n", filename, filename).as_bytes());

            acc.open_new_file(*filenames.get(filename).unwrap());
        } else if l.starts_with(&args.hunk) {
            acc.dump_hunk(&mut patch);
            acc.open_new_hunk(l[args.hunk.len()..].trim());
        } else {
            acc.push_line(l);
        }
    }
    acc.dump_hunk(&mut patch);

    eprintln!("{}", std::str::from_utf8(&patch).unwrap());

    let mut apply = Command::new("git")
        .args(&["apply", "-"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = apply.stdin.take().unwrap();
    stdin.write_all(&patch).unwrap();

    // apply.wait().unwrap();
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

    fn dump_hunk(&mut self, patch: &mut Vec<u8>) {
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

        patch.extend_from_slice(
            format!(
                "@@ -{},{} +{},{} @@\n",
                original_pos,
                original_len,
                (original_pos as isize + self.pos_diff) as usize,
                self.edited_len
            )
            .as_bytes(),
        );
        for l in content.lines() {
            patch.push(b'-');
            patch.extend_from_slice(l.as_bytes());
            patch.push(b'\n');
        }
        for l in self.buf.lines() {
            patch.push(b'+');
            patch.extend_from_slice(l.as_bytes());
            patch.push(b'\n');
        }

        self.pos_diff += self.edited_len as isize;
        self.pos_diff -= original_len as isize;
        self.open_new_hunk("");
    }
}
