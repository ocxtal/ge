use crate::git::{Git, GrepOptions, GrepResult};
use anyhow::Result;
use clap::Parser;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::ops::Range;

#[derive(Debug, Parser)]
pub struct HunkOptions {
    #[clap(
        short = 'C',
        long,
        value_name = "N",
        help = "Include <N> additional lines before and after matches"
    )]
    context: Option<usize>,

    #[clap(
        short = 'B',
        long = "before-context",
        value_name = "N",
        help = "Include <N> additional lines before matches"
    )]
    before: Option<usize>,

    #[clap(
        short = 'A',
        long = "after-context",
        value_name = "N",
        help = "Include <N> additional lines after matches"
    )]
    after: Option<usize>,

    #[clap(
        short = 'H',
        long = "head",
        value_name = "N",
        help = "Edit <N> lines from the head of files that have matches"
    )]
    head: Option<usize>,

    #[clap(
        long = "with",
        value_name = "PATTERN",
        help = "Filter out files that don't have the PATTERN"
    )]
    with: Option<String>,

    #[clap(
        long = "without",
        value_name = "PATTERN",
        help = "Filter out files that have the PATTERN"
    )]
    without: Option<String>,

    #[clap(
        long = "to",
        value_name = "PATTERN",
        help = "Extend match downward until the first hit of PATTERN with the same indentation level"
    )]
    to: Option<String>,
}

trait MatchExtender {
    fn filter_files(&mut self, secondary: &GrepResult, invert: bool) -> Result<()>;
    fn collect_head(&mut self, n_lines: usize) -> Result<()>;
    fn extend_to_another(&mut self, to: &GrepResult) -> Result<()>;
    fn extend_by_lines(&mut self, up: usize, down: usize) -> Result<()>;
    fn filter_overlaps(&mut self) -> Result<()>;
}

impl MatchExtender for GrepResult {
    fn filter_files(&mut self, secondary: &GrepResult, invert: bool) -> Result<()> {
        let file_ids: HashSet<_> = secondary.files.iter().map(|x| x.to_string()).collect();
        self.hits = self
            .hits
            .iter()
            .filter_map(|x| {
                if invert ^ file_ids.get(&self.files[x.file_id]).is_some() {
                    Some(*x)
                } else {
                    None
                }
            })
            .collect();

        Ok(())
    }

    fn collect_head(&mut self, n_lines: usize) -> Result<()> {
        for hit in &mut self.hits {
            hit.from = 0;
            hit.n_lines = n_lines;
        }

        self.hits.sort();
        self.hits.dedup();

        Ok(())
    }

    fn extend_to_another(&mut self, to: &GrepResult) -> Result<()> {
        let mut it = to.hits.iter().peekable();

        for hit in &mut self.hits {
            // skip_while
            // note: files (filenames) are sorted in the ascending order so it's safe to
            // iterate this loop with the >= comparator.
            while let Some(x) = it.peek() {
                if (&to.files[x.file_id], x.from) >= (&self.files[hit.file_id], hit.from)
                    && x.level == hit.level
                {
                    break;
                }
                it.next().unwrap();
            }

            if it.peek().is_none() {
                break;
            }
            let next = it.peek().unwrap();
            if to.files[next.file_id] != self.files[hit.file_id] {
                continue;
            }

            let next = it.next().unwrap();
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
        let mut matches = git.grep(pattern, true, grep_opts)?;

        // first filter files out
        if let Some(pattern) = &hunk_opts.with {
            let with = git.grep(pattern, false, grep_opts)?;
            matches.filter_files(&with, false)?;
        }

        if let Some(pattern) = &hunk_opts.without {
            let without = git.grep(pattern, false, grep_opts)?;
            matches.filter_files(&without, true)?;
        }

        // move hits to the head if --head exists
        if let Some(head) = &hunk_opts.head {
            matches.collect_head(*head)?;
        }

        // extend to secondary hit locations
        if let Some(pattern) = &hunk_opts.to {
            let to = git.grep(pattern, false, grep_opts)?;
            matches.extend_to_another(&to)?;
        }

        // lastly extend hits upward and downward
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
    use crate::{Git, GrepOptions, HunkOptions, Hunks};
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

        let hunks = Hunks::collect(&git, "assert", &grep_opts, opts!("ge --head 2")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 0);
        assert_eq!(hunks.hunks[0].2.len(), 2);

        let hunks = Hunks::collect(&git, "fox", &grep_opts, opts!("ge --head 3")).unwrap();
        assert_eq!(hunks.files.len(), 1);
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 0);
        assert_eq!(hunks.hunks[0].2.len(), 3);

        let hunks = Hunks::collect(&git, "assert", &grep_opts, opts!("ge --with fn")).unwrap();
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 2);
        assert_eq!(hunks.hunks[0].2.len(), 1);

        let hunks = Hunks::collect(&git, "assert", &grep_opts, opts!("ge --with xyzxyz")).unwrap();
        assert_eq!(hunks.hunks.len(), 0);

        let hunks = Hunks::collect(&git, "assert", &grep_opts, opts!("ge --without fn")).unwrap();
        assert_eq!(hunks.hunks.len(), 0);

        let hunks =
            Hunks::collect(&git, "assert", &grep_opts, opts!("ge --without xyzxyz")).unwrap();
        assert_eq!(hunks.hunks.len(), 1);
        assert_eq!(hunks.hunks[0].1, 2);
        assert_eq!(hunks.hunks[0].2.len(), 1);
    }
}
