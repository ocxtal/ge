mod editor;
mod git;
mod pager;
mod patch;

use anyhow::{Context, Result};
use clap::Parser;
use std::io::{BufReader, BufWriter, Write};

use crate::editor::Editor;
use crate::git::{Git, GrepArgs};
use crate::pager::Pager;
use crate::patch::{HalfDiffConfig, PatchBuilder};

#[derive(Parser, Debug)]
#[clap(author, version = "0.0.1", about = "grep and edit your files in bulk", long_about = None)]
struct Args {
    #[clap(help = "Pattern to search")]
    pattern: String,

    #[clap(
        short = 'C',
        long,
        name = "N",
        help = "Show <N> lines before and after matches"
    )]
    context: Option<usize>,

    #[clap(
        short = 'B',
        long = "before-context",
        name = "N",
        help = "Show <N> lines before matches"
    )]
    before: Option<usize>,

    #[clap(
        short = 'A',
        long = "after-context",
        name = "N",
        help = "Show <N> lines after matches"
    )]
    after: Option<usize>,

    #[clap(short, long, help = "Show matches and exit")]
    preview: bool,

    #[clap(long, default_value = "+++", help = "Use <HEADER> for header markers")]
    header: String,

    #[clap(long, default_value = "@@", help = "Use <HUNK> for hunk markers")]
    hunk: String,

    #[clap(short, long, help = "Use <EDITOR> to edit matches [default: vi]")]
    editor: Option<String>,

    #[clap(long, help = "Use <PAGER> to preview matches [default: less]")]
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

    // create git object
    let git = Git::new()?;

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
    if args.preview {
        let mut pager = Pager::new(&arg_or_env_or_default(&args.pager, "PAGER", "less"))?;
        {
            let mut writer = BufWriter::new(&mut pager);
            gen.write_halfdiff(&mut writer)?;
            writer.flush()?;
        }
        pager.wait()?;

        return Ok(());
    }

    let mut editor = Editor::new(&arg_or_env_or_default(&args.editor, "EDITOR", "vi"))?;
    {
        let mut writer = BufWriter::new(&mut editor);
        gen.write_halfdiff(&mut writer)?;
        writer
            .flush()
            .context("failed to flush the tempfile. aborting.")?;
    }

    // wait for the user...
    editor.wait()?;

    // read the edit result, and parse it into a unified diff
    let mut reader = BufReader::new(&mut editor);
    let patch = gen.read_halfdiff(&mut reader)?;

    // then apply the patch
    git.apply(&patch)?;

    // we've done all
    Ok(())
}
