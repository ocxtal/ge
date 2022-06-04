use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::io::{Read, Write};

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

pub struct HalfDiffConfig<'a> {
    pub header: &'a str,
    pub hunk: &'a str,
}

pub struct PatchBuilder<'a> {
    config: &'a HalfDiffConfig<'a>,
    files: HashMap<String, usize>,
    lines: HashMap<(usize, usize), (usize, String)>,
}

impl<'a> PatchBuilder<'a> {
    pub fn from_grep(config: &'a HalfDiffConfig, raw: &str) -> Result<Self> {
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
                return Err(anyhow!("unexpected grep line: {}. aborting.", l));
            }

            if v[0] == "" {
                debug_assert!(v[1] == "" && v[2] == "");
                (prev_id, prev_pos, prev_base_pos) = (0, 0, 0);
                continue;
            }

            let filename = &v[0];
            let ln: usize = v[1]
                .parse()
                .with_context(|| format!("broken grep line number: {}. aborting.", &v[1]))?;
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

    pub fn write_halfdiff(&self, drain: &mut dyn Write) -> Result<()> {
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

    pub fn read_halfdiff(&self, src: &mut dyn Read) -> Result<String> {
        let mut buf = Vec::new();
        src.read_to_end(&mut buf)
            .context("failed to read the edit result. aborting.")?;

        let mut patch = String::new();
        let mut acc = HunkAccumulator::new(&self.lines);

        let diff = std::str::from_utf8(&buf)
            .context("failed parse the edit result as a UTF-8 string. aborting.")?;

        for l in diff.lines() {
            if l.starts_with(&self.config.header) {
                acc.dump_hunk(&mut patch);

                let filename = l[self.config.header.len()..].trim();
                patch.push_str(&format!("--- a/{}\n+++ b/{}\n", filename, filename));

                let filename = self.files.get(filename).with_context(|| {
                    format!(
                        "got an invalid filename {:?} in the edit result. aborting.",
                        filename
                    )
                })?;
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
