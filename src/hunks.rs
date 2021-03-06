use crate::git::{Git, GrepOptions, GrepResult};
use anyhow::Result;
use clap::Parser;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::ops::Range;

#[derive(Debug, Parser)]
pub struct HunkOptions {
    #[clap(
        short = 'C',
        long,
        name = "N",
        help = "Include <N> additional lines before and after matches"
    )]
    context: Option<usize>,

    #[clap(
        short = 'B',
        long = "before-context",
        name = "B",
        help = "Include <B> additional lines before matches"
    )]
    before: Option<usize>,

    #[clap(
        short = 'A',
        long = "after-context",
        name = "A",
        help = "Include <A> additional lines after matches"
    )]
    after: Option<usize>,

    #[clap(
        long = "to",
        name = "TO",
        help = "Extend match downward until the first hit of TO"
    )]
    to: Option<String>,
}

trait MatchExtender {
    fn extend_to_another(&mut self, to: &GrepResult) -> Result<()>;
    fn extend_by_lines(&mut self, up: usize, down: usize) -> Result<()>;
    fn filter_overlaps(&mut self) -> Result<()>;
}

impl MatchExtender for GrepResult {
    fn extend_to_another(&mut self, to: &GrepResult) -> Result<()> {
        let mut to = to.hits.iter().peekable();

        for hit in &mut self.hits {
            // skip_while
            while let Some(x) = to.peek() {
                if (x.file_id, x.from) >= (hit.file_id, hit.from) {
                    break;
                }
                to.next().unwrap();
            }

            if to.peek().is_none() {
                break;
            }
            if to.peek().unwrap().file_id != hit.file_id {
                continue;
            }

            let next = to.next().unwrap();
            hit.n_lines = next.from + next.n_lines - hit.from;
        }
        Ok(())
    }

    fn extend_by_lines(&mut self, up: usize, down: usize) -> Result<()> {
        for hit in &mut self.hits {
            let end = hit.from + hit.n_lines + down;
            let start = hit.from.saturating_sub(up);

            hit.from = start;
            hit.n_lines = end - start;
        }
        Ok(())
    }

    fn filter_overlaps(&mut self) -> Result<()> {
        let mut n_drop = 0;
        for i in 1..self.hits.len() {
            let (dst, srcs) = self.hits.split_at_mut(i - n_drop);

            let dst = dst.last_mut().unwrap();
            let src = &srcs[n_drop];

            if dst.file_id == src.file_id && dst.from + dst.n_lines >= src.from {
                dst.n_lines = src.from + src.n_lines - dst.from;
                n_drop += 1;
            }

            if n_drop < srcs.len() {
                srcs[0] = srcs[n_drop];
            }
        }
        self.hits.truncate(self.hits.len() - n_drop);

        Ok(())
    }
}

#[derive(Debug)]
pub struct Hunks {
    pub files: Vec<String>,
    pub hunks: Vec<(usize, usize, Vec<String>)>,
}

impl Hunks {
    pub fn collect(
        git: &Git,
        pattern: &str,
        grep_opts: &GrepOptions,
        hunk_opts: &HunkOptions,
    ) -> Result<Self> {
        let matches = Self::collect_matches(git, pattern, grep_opts, hunk_opts)?;
        Self::collect_hunks(matches)
    }

    fn collect_matches(
        git: &Git,
        pattern: &str,
        grep_opts: &GrepOptions,
        hunk_opts: &HunkOptions,
    ) -> Result<GrepResult> {
        let mut matches = git.grep(pattern, grep_opts)?;

        if let Some(pattern) = &hunk_opts.to {
            let to = git.grep(pattern, grep_opts)?;
            matches.extend_to_another(&to)?;
        }

        if let Some(c) = hunk_opts.context {
            matches.extend_by_lines(c, c)?;
        } else {
            let b = hunk_opts.before.unwrap_or(0);
            let a = hunk_opts.after.unwrap_or(0);
            if a != 0 || b != 0 {
                matches.extend_by_lines(b, a)?;
            }
        }

        matches.filter_overlaps()?;

        Ok(matches)
    }

    fn collect_hunks(matches: GrepResult) -> Result<Self> {
        let mut hunks = Vec::new();

        // group_by iterator
        let mut from = 0;
        for i in 1..matches.hits.len() {
            let (first, next) = matches.hits.split_at(i);
            let first = &first[from];
            let next = &next[0];

            if first.file_id == next.file_id {
                continue;
            }

            Self::collect_hunks_from_file(&matches, from..i, &mut hunks)?;
            from = i;
        }

        if from < matches.hits.len() {
            Self::collect_hunks_from_file(&matches, from..matches.hits.len(), &mut hunks)?;
        }

        Ok(Hunks {
            files: matches.files,
            hunks,
        })
    }

    fn collect_hunks_from_file(
        matches: &GrepResult,
        range: Range<usize>,
        hunks: &mut Vec<(usize, usize, Vec<String>)>,
    ) -> Result<()> {
        let file_id = matches.hits[range.start].file_id;
        let f = BufReader::new(File::open(&matches.files[file_id])?);

        let mut it = f.lines().enumerate().peekable();

        for hit in &matches.hits[range] {
            // skip_while
            while let Some(&(x, _)) = it.peek() {
                if x >= hit.from {
                    break;
                }
                it.next().unwrap().1?;
            }

            let lines = Self::collect_lines(&mut it, hit.n_lines)?;
            hunks.push((file_id, hit.from, lines));
        }

        Ok(())
    }

    fn collect_lines<I>(it: &mut I, n_lines: usize) -> Result<Vec<String>>
    where
        I: Iterator<Item = (usize, Result<std::string::String, std::io::Error>)>,
    {
        let mut acc = Vec::new();

        for _ in 0..n_lines {
            if let Some((_, line)) = it.next() {
                acc.push(line?.to_string());
            } else {
                break;
            }
        }

        Ok(acc)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Git, GrepOptions, Hunks, HunkOptions};
    use clap::Parser;

    #[test]
    fn test_collect() {
        macro_rules! opts {
            ( $args: expr ) => {
                &HunkOptions::parse_from($args.split_whitespace())
            };
        }

        let git = Git::new().unwrap();
        let grep_opts = GrepOptions::parse_from("ge -y tests".split_whitespace());

        let hunks = Hunks::collect(&git, "assert_eq", &grep_opts, opts!("ge")).unwrap();
        assert_eq!(hunks.files.len(), 0);
        assert_eq!(hunks.hunks.len(), 0);

        let hunks = Hunks::collect(&git, "assert", &grep_opts, opts!("ge")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 2);
        assert_eq!(hunks.hunks[0].2.len(), 1);

        let hunks = Hunks::collect(&git, "assert", &grep_opts, opts!("ge -B2")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 0);
        assert_eq!(hunks.hunks[0].2.len(), 3);

        let hunks = Hunks::collect(&git, "assert", &grep_opts, opts!("ge -B4")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 0);
        assert_eq!(hunks.hunks[0].2.len(), 3);

        let hunks = Hunks::collect(&git, "assert", &grep_opts, opts!("ge -A2")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 2);
        assert_eq!(hunks.hunks[0].2.len(), 2);

        let hunks = Hunks::collect(&git, "assert", &grep_opts, opts!("ge -C0")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 2);
        assert_eq!(hunks.hunks[0].2.len(), 1);

        let hunks = Hunks::collect(&git, "assert", &grep_opts, opts!("ge -C1")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 1);
        assert_eq!(hunks.hunks[0].2.len(), 3);

        let hunks = Hunks::collect(&git, "assert", &grep_opts, opts!("ge -C5")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 0);
        assert_eq!(hunks.hunks[0].2.len(), 4);

        let hunks = Hunks::collect(&git, "fn", &grep_opts, opts!("ge --to )")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 1);
        assert_eq!(hunks.hunks[0].2.len(), 1);

        let hunks = Hunks::collect(&git, "fn", &grep_opts, opts!("ge --to }")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 1);
        assert_eq!(hunks.hunks[0].2.len(), 3);

        let hunks = Hunks::collect(&git, "fox", &grep_opts, opts!("ge")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 2);

        let hunks = Hunks::collect(&git, "fox", &grep_opts, opts!("ge -C5")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 1);

        let hunks = Hunks::collect(&git, "f.\\+", &grep_opts, opts!("ge")).unwrap();
        assert_eq!(hunks.files.len(), 2);
        assert_eq!(hunks.hunks.len(), 3);

        let hunks = Hunks::collect(&git, "f.\\+", &grep_opts, opts!("ge -C5")).unwrap();
        assert_eq!(hunks.files.len(), 2);
        assert_eq!(hunks.hunks.len(), 2);
    }
}
