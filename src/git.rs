use anyhow::{anyhow, Context, Result};
use clap::{ArgEnum, Parser};
use std::io::Write;
use std::process::{Command, Stdio};

pub struct Git;

#[derive(Copy, Clone, Debug, ArgEnum)]
enum GrepMode {
    Fixed,
    Extended,
    Basic,
    Pcre,
}

#[derive(Debug, Parser)]
pub struct GrepOptions {
    #[clap(
        arg_enum,
        short = 'M',
        long = "mode",
        default_value = "basic",
        help = "Regex mode"
    )]
    mode: GrepMode,

    #[clap(
        short = 'W',
        long = "function-context",
        help = "Extend match to the entire function"
    )]
    function: bool,

    #[clap(short = 'i', long = "ignore-case", help = "Case-insensitive search")]
    ignore_case: bool,

    #[clap(short = 'w', long = "word-regexp", help = "Match at word boundaries")]
    word_boundary: bool,

    #[clap(
        long = "max-depth",
        value_name = "N",
        help = "Maximum directory depth to search [default: inf]"
    )]
    max_depth: Option<usize>,

    #[clap(
        short = 'y',
        long,
        value_name = "PATHSPEC",
        help = "Files to search (in pathspec; multiple allowed)"
    )]
    only: Vec<String>,

    #[clap(
        short = 'x',
        long,
        value_name = "PATHSPEC",
        help = "Files to exclude in search (in pathspec; multiple allowed)"
    )]
    exclude: Vec<String>,
}

impl Git {
    pub fn new() -> Result<Self> {
        // check the availability of the git command
        let output = Command::new("git")
            .args(["--version"])
            .output()
            .context("\"git\" command not found.")?;
        assert!(output.status.success());

        Ok(Git)
    }

    fn expand_options(&self, opts: &GrepOptions, args: &mut Vec<String>) {
        args.push(match opts.mode {
            GrepMode::Fixed => "--fixed-strings".to_string(),
            GrepMode::Basic => "--basic-regexp".to_string(),
            GrepMode::Extended => "--extended-regexp".to_string(),
            GrepMode::Pcre => "--perl-regexp".to_string(),
        });

        if opts.function {
            args.push("--function-context".to_string());
        }
        if opts.ignore_case {
            args.push("--ignore-case".to_string());
        }
        if opts.word_boundary {
            args.push("--word-regexp".to_string());
        }
        if let Some(depth) = opts.max_depth {
            args.push(format!("--max-depth={}", depth));
        }
    }

    pub fn grep(&self, pattern: &str, opts: &GrepOptions) -> Result<GrepResult> {
        // compose arguments
        let mut args = vec![
            "grep".to_string(),
            "--color=never".to_string(),
            "--line-number".to_string(),
            "-I".to_string(),     // exclude binary files
            "--null".to_string(), // for unambiguous delimiters
        ];

        self.expand_options(opts, &mut args);
        args.push(pattern.to_string());

        if !opts.only.is_empty() || !opts.exclude.is_empty() {
            args.push("--".to_string());
        }

        // append pathspec if "--only" exists
        if !opts.only.is_empty() {
            let only = opts
                .only
                .iter()
                .flat_map(|x| x.split(',').collect::<Vec<_>>());
            for pattern in only {
                args.push(pattern.to_string());
            }
        }

        // append pathspec if "--exclude" exists
        if !opts.exclude.is_empty() {
            let exclude = opts
                .exclude
                .iter()
                .flat_map(|x| x.split(',').collect::<Vec<_>>());
            for pattern in exclude {
                args.push(format!(":!{}", pattern));
            }
        }

        // run git-grep then parse the output as a utf-8 string
        let output = Command::new("git")
            .args(&args)
            .output()
            .context("failed to get output of \"git grep\". aborting.")?;
        let output = String::from_utf8(output.stdout).context(
            "failed to interpret the output of \"git grep\" as a UTF-8 string. aborting.",
        )?;

        GrepResult::from_raw(&output)
    }

    pub fn apply(&self, patch: &str) -> Result<()> {
        let mut apply = Command::new("git")
            .args(["apply", "--unidiff-zero", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("failed to run \"git apply\". aborting.")?;

        // we expect it's dropped after use (it sends EOF)
        {
            let mut stdin = apply.stdin.take().unwrap();
            stdin.write_all(patch.as_bytes()).unwrap();
        }

        // make sure patch was successful
        let code = apply
            .wait()
            .context("\"git apply\" unexpectedly exited. aborting.")?;
        if !code.success() {
            return Err(anyhow!(
                "\"git apply\" returned an error{}. aborting.",
                code
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GrepHit {
    pub file_id: usize,
    pub from: usize,
    pub n_lines: usize,
}

#[derive(Debug)]
pub struct GrepResult {
    pub files: Vec<String>,
    pub hits: Vec<GrepHit>,
}

impl GrepResult {
    fn parse_line(line: &str) -> Result<(&str, usize)> {
        // find two '\0's
        let pos = line.find('\0').with_context(|| {
            format!(
                "failed to find filename-linenumber delimiter in {:?}. aborting.",
                line
            )
        })?;
        let (filename, rem) = line.split_at(pos);

        let pos = rem[1..].find('\0').with_context(|| {
            format!(
                "failed to find linenumber-body delimiter in {:?}. aborting.",
                line
            )
        })?;

        let (at, _) = rem[1..].split_at(pos);
        let at: usize = at
            .parse()
            .with_context(|| format!("broken grep line number: {}. aborting.", at))?;
        debug_assert!(at > 0);

        Ok((filename, at - 1))
    }

    fn from_raw(raw: &str) -> Result<GrepResult> {
        let mut bin = GrepResult {
            files: Vec::new(),
            hits: Vec::new(),
        };

        for line in raw.trim().lines() {
            if line == "--" {
                continue;
            }

            let (filename, at) = Self::parse_line(line)?;

            if bin.files.is_empty() || bin.files.last().unwrap() != filename {
                bin.files.push(filename.to_string());
            }

            let file_id = bin.files.len() - 1;
            if let Some(last_hit) = bin.hits.last_mut() {
                if last_hit.file_id == file_id && last_hit.from + last_hit.n_lines == at {
                    last_hit.n_lines += 1;
                    continue;
                }
            }

            bin.hits.push(GrepHit {
                file_id,
                from: at,
                n_lines: 1,
            });
        }
        Ok(bin)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Git, GrepOptions};
    use clap::Parser;

    #[test]
    fn test_new() {
        assert!(Git::new().is_ok());
    }

    #[test]
    fn test_grep() {
        macro_rules! opts {
            ( $args: expr ) => {
                &GrepOptions::parse_from($args.split_whitespace())
            };
        }

        // assume tests/quick.txt exists
        let git = Git::new().unwrap();

        // "ge" is a placeholder for a command name
        let output = git.grep("fox", opts!("ge")).unwrap();
        assert!(output.hits.len() >= 2);

        let output = git.grep("fox", opts!("ge -y tests/*.txt")).unwrap();
        assert_eq!(output.hits.len(), 2);

        let output = git.grep("fox", opts!("ge -x tests/*.txt -x src")).unwrap();
        assert_eq!(output.hits.len(), 0);

        let output = git.grep("fox", opts!("ge --max-depth 0")).unwrap();
        assert_eq!(output.hits.len(), 0);

        let output = git.grep("fox", opts!("ge --max-depth 1")).unwrap();
        assert!(output.hits.len() >= 2);

        let output = git.grep("FOX", opts!("ge -y tests/*.txt -i")).unwrap();
        assert_eq!(output.hits.len(), 2);

        let output = git.grep("quic", opts!("ge -y tests/*.txt")).unwrap();
        assert_eq!(output.hits.len(), 1);

        let output = git.grep("quic", opts!("ge -y tests/*.txt -w")).unwrap();
        assert_eq!(output.hits.len(), 0);

        // --mode
        let output = git
            .grep("(fox)|(dog)", opts!("ge --mode=basic -y tests/*.txt"))
            .unwrap();
        assert_eq!(output.hits.len(), 0);

        let output = git
            .grep(
                "\\(fox\\)\\|\\(dog\\)",
                opts!("ge --mode=basic -y tests/*.txt"),
            )
            .unwrap();
        assert!(output.hits.len() > 0);

        let output = git
            .grep("(fox)|(dog)", opts!("ge --mode=extended -y tests/*.txt"))
            .unwrap();
        assert!(output.hits.len() > 0);

        let output = git
            .grep("(fox)|(dog)", opts!("ge --mode=extended -y tests/*.txt"))
            .unwrap();
        assert!(output.hits.len() > 0);

        // --function-context
        let output = git.grep("assert", opts!("ge -y tests/*.rs")).unwrap();
        assert_eq!(output.hits.len(), 1);
        assert_eq!(output.hits[0].n_lines, 1);

        let output = git
            .grep("assert", opts!("ge --function-context -y tests/*.rs"))
            .unwrap();
        assert_eq!(output.hits.len(), 1);
        assert!(output.hits[0].n_lines >= 3); // workaround for old versions of git that excludes `#[test]`
    }

    // TODO: git.apply
}
