mod editor;
mod git;
mod hunks;
mod pager;
mod patch;

use anyhow::{Context, Result};
use clap::Parser;
use std::io::{BufWriter, Write};

use crate::editor::Editor;
use crate::git::{Git, GrepOptions};
use crate::hunks::{HunkOptions, Hunks};
use crate::pager::Pager;
use crate::patch::{HalfDiffConfig, PatchBuilder};

#[derive(Debug, Parser)]
#[clap(author, version, about = "grep and edit git-tracked files in bulk", long_about = None)]
struct Args {
    #[clap(help = "Pattern to search")]
    pattern: String,

    #[clap(flatten)]
    grep_opts: GrepOptions,

    #[clap(flatten)]
    hunk_opts: HunkOptions,

    #[clap(short, long, help = "Show matches and exit")]
    preview: bool,

    #[clap(
        long,
        value_name = "MARKER",
        help = "Use <MARKER> for header markers [default: +++]"
    )]
    header: Option<String>,

    #[clap(
        long,
        value_name = "MARKER",
        help = "Use <MARKER> for hunk markers [default: @@]"
    )]
    hunk: Option<String>,

    #[clap(short, long, help = "Use <EDITOR> to edit matches [default: vi]")]
    editor: Option<String>,

    #[clap(short, long, help = "Get edited halfdiff from stdout")]
    stdout: bool,

    #[clap(long, help = "Use <PAGER> to preview matches [default: less -F]")]
    pager: Option<String>,
}

fn arg_or_env_or_default(arg: &Option<String>, env: &str, default: &str) -> String {
    if let Some(arg) = arg {
        return arg.clone();
    }
    if let Ok(env) = std::env::var(env) {
        return env;
    }
    default.to_string()
}

fn main() -> Result<()> {
    let args = Args::parse();

    // create git object, run git-grep to collect matches
    let git = Git::new()?;
    let hunks = Hunks::collect(&git, &args.pattern, &args.grep_opts, &args.hunk_opts)?;

    // parse the result
    let config = &HalfDiffConfig {
        header: args.header.as_deref(),
        hunk: args.hunk.as_deref(),
    };
    let builder = PatchBuilder::from_hunks(config, hunks)?;

    // convert the git-grep result (hit locations) into "halfdiff" that will be edited by the user
    if args.preview {
        let mut pager = Pager::new(&arg_or_env_or_default(&args.pager, "PAGER", "less -F"))?;
        {
            let mut writer = BufWriter::new(&mut pager);
            builder.write_halfdiff(&mut writer)?;
            writer.flush()?;
        }
        pager.wait()?;

        return Ok(());
    }

    let mut editor = Editor::new(
        &arg_or_env_or_default(&args.editor, "EDITOR", "vi"),
        args.stdout,
    )?;
    {
        let mut writer = BufWriter::new(&mut editor);
        builder.write_halfdiff(&mut writer)?;
        writer
            .flush()
            .context("failed to flush the tempfile. aborting.")?;
    }

    // wait for the user...
    editor.wait()?;

    // read the edit result, and parse it into a unified diff
    let patch = builder.parse_halfdiff(editor.get_buf())?;

    // then apply the patch
    if !patch.is_empty() {
        git.apply(&patch)?;
    }

    // we've done all
    Ok(())
}
