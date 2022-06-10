use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::io::{Read, Write};

struct LineAccumulator<'a, 'b> {
    id: usize,
    hunk: &'a str,
    buf: String,
    edited_len: usize,
    pos_diff: isize,
    original: &'b HashMap<(usize, usize), Vec<String>>,
}

impl<'a, 'b> LineAccumulator<'a, 'b> {
    fn new(original: &'b HashMap<(usize, usize), Vec<String>>) -> Self {
        LineAccumulator {
            id: 0,
            hunk: "",
            buf: String::new(),
            edited_len: 0,
            pos_diff: 0,
            original,
        }
    }

    fn is_empty(&self) -> bool {
        self.id == 0 || self.hunk.is_empty()
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
        assert!(!self.is_empty());

        self.buf.push_str(line);
        self.buf.push('\n');
        self.edited_len += 1;
    }

    fn is_edited(&self, original_lines: &[String]) -> bool {
        for (o, t) in self
            .buf
            .lines()
            .chain(std::iter::repeat(""))
            .zip(original_lines.iter())
        {
            if o != t {
                return true;
            }
        }
        false
    }

    fn dump_hunk(&mut self, acc: &mut HunkAccumulator) -> Result<()> {
        if self.is_empty() {
            // clear the state
            self.open_new_hunk("");
            return Ok(());
        }

        let hunk: Vec<_> = self.hunk.split(',').collect();
        let original_pos = hunk[0].parse().unwrap();
        let original_lines = self.original.get(&(self.id, original_pos)).unwrap();

        if !self.is_edited(original_lines) {
            // clear the state
            self.open_new_hunk("");
            return Ok(());
        }

        let mut buf = String::new();
        writeln!(
            &mut buf,
            "@@ -{},{} +{},{} @@",
            original_pos,
            original_lines.len(),
            (original_pos as isize + self.pos_diff) as usize,
            self.edited_len
        )?;
        for l in original_lines {
            buf.push('-');
            buf.push_str(l);
            buf.push('\n');
        }
        for l in self.buf.lines() {
            buf.push('+');
            buf.push_str(l);
            buf.push('\n');
        }
        acc.push_hunk(buf.as_str());

        self.pos_diff += self.edited_len as isize;
        self.pos_diff -= original_lines.len() as isize;
        self.open_new_hunk("");

        Ok(())
    }
}

struct HunkAccumulator {
    header_len: usize,
    buf: String,
}

impl HunkAccumulator {
    fn new() -> Self {
        HunkAccumulator {
            buf: String::new(),
            header_len: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.header_len == self.buf.len()
    }

    fn open_new_patch(&mut self, filename: &str) {
        let header = format!("--- a/{}\n+++ b/{}\n", filename, filename);
        self.header_len = header.len();
        self.buf = header;
    }

    fn push_hunk(&mut self, hunk: &str) {
        self.buf.push_str(hunk);
    }

    fn dump_patch(&mut self, acc: &mut String) {
        if self.is_empty() {
            return;
        }

        acc.push_str(&self.buf);
        self.header_len = 0;
    }
}

pub struct HalfDiffConfig<'a> {
    pub header: Option<&'a str>,
    pub hunk: Option<&'a str>,
}

pub struct PatchBuilder {
    header_marker: String,
    hunk_marker: String,
    header_collision_avoidance: bool,
    hunk_collision_avoidance: bool,
    files: HashMap<String, usize>,
    raw_hunks: HashMap<(usize, usize), Vec<String>>,
}

impl PatchBuilder {
    pub fn from_grep(config: &HalfDiffConfig, raw: &str) -> Result<Self> {
        let header_marker = config.header.map_or("+++".to_string(), |x| x.to_string());
        let hunk_marker = config.hunk.map_or("@@".to_string(), |x| x.to_string());

        let mut locs = PatchBuilder {
            header_marker,
            hunk_marker,
            header_collision_avoidance: config.header.is_none(),
            hunk_collision_avoidance: config.hunk.is_none(),
            files: HashMap::new(),
            raw_hunks: HashMap::new(),
        };

        locs.parse_grep(raw)?;
        locs.avoid_collision()?;
        Ok(locs)
    }

    fn scan_lines(&self, marker: &str) -> bool {
        for lines in self.raw_hunks.values() {
            for line in lines {
                if line.starts_with(marker) {
                    return true;
                }
            }
        }
        false
    }

    fn avoid_collision(&mut self) -> Result<()> {
        // header
        for i in 0..17 {
            if !self.scan_lines(&self.header_marker) {
                break;
            }
            if i == 16 || !self.header_collision_avoidance {
                return Err(anyhow!(
                    "failed to avoid collision with the header marker {:?}. aborting.",
                    self.header_marker
                ));
            }

            self.header_marker.push('+');
        }

        // hunk
        for i in 0..17 {
            if !self.scan_lines(&self.hunk_marker) {
                break;
            }
            if i == 16 || !self.hunk_collision_avoidance {
                return Err(anyhow!(
                    "failed to avoid collision with the hunk marker {:?}. aborting.",
                    self.hunk_marker
                ));
            }

            self.hunk_marker.push('@');
        }
        Ok(())
    }

    fn parse_grep(&mut self, raw: &str) -> Result<()> {
        let mut prev_id = 0;
        let mut prev_pos = 0;
        let mut prev_base_pos = 0;
        for l in raw.trim().lines() {
            let v: Vec<_> = l.splitn(3, &[':', '-', '='][..]).collect();
            if v.len() != 3 {
                return Err(anyhow!("unexpected grep line: {}. aborting.", l));
            }

            if v[0].is_empty() {
                debug_assert!(v[1].is_empty() && v[2].is_empty());
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
                self.raw_hunks.entry((id, prev_base_pos)).and_modify(|e| {
                    e.push(v[2].to_string()); // we may need to add a prefix here
                });
            } else {
                self.raw_hunks.insert((id, ln), vec![v[2].to_string()]);
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
        let mut keys: Vec<_> = self.raw_hunks.keys().collect();
        keys.sort();

        let mut prev_id = 0;
        for &(id, pos) in keys {
            if prev_id != id {
                let filename = index.get(&id).unwrap();
                drain.write_all(format!("{} {}\n", self.header_marker, filename).as_bytes())?;
                prev_id = id;
            }

            let lines = self.raw_hunks.get(&(id, pos)).unwrap();

            let mut acc = format!("{} {},{}\n", self.hunk_marker, pos, lines.len());
            for line in lines {
                acc.push_str(line);
                acc.push('\n');
            }

            drain.write_all(acc.as_bytes())?;
        }

        Ok(())
    }

    pub fn read_halfdiff(&self, src: &mut dyn Read) -> Result<String> {
        let mut buf = Vec::new();
        src.read_to_end(&mut buf)
            .context("failed to read the edit result. aborting.")?;

        let mut patch = String::new();
        let mut hunks = HunkAccumulator::new();
        let mut lines = LineAccumulator::new(&self.raw_hunks);

        let diff = std::str::from_utf8(&buf)
            .context("failed parse the edit result as a UTF-8 string. aborting.")?;

        for l in diff.lines() {
            if l.starts_with(&self.header_marker) {
                lines.dump_hunk(&mut hunks)?;
                hunks.dump_patch(&mut patch);

                let filename = l[self.header_marker.len()..].trim();
                let id = self.files.get(filename).with_context(|| {
                    format!(
                        "got an invalid filename {:?} in the edit result. aborting.",
                        filename
                    )
                })?;

                hunks.open_new_patch(filename);
                lines.open_new_file(*id);
            } else if l.starts_with(&self.hunk_marker) {
                lines.dump_hunk(&mut hunks)?;
                lines.open_new_hunk(l[self.hunk_marker.len()..].trim());
            } else {
                lines.push_line(l);
            }
        }
        lines.dump_hunk(&mut hunks)?;
        hunks.dump_patch(&mut patch);

        Ok(patch)
    }
}
