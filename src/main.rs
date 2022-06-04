mod editor;
mod git;
mod pager;
mod patch;

use anyhow::{Context, Result};
use clap::Parser;
use std::env::var;
use std::io::{BufReader, BufWriter, Write};

use crate::editor::Editor;
use crate::git::{Git, GrepArgs};
use crate::pager::Pager;
use crate::patch::{HalfDiffConfig, PatchBuilder};

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

    #[clap(long)]
    pager: Option<String>,
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
        let mut writer: Box<dyn Write> = if args.preview {
            let pager = Pager::new(
                args.pager
                    .as_deref()
                    .unwrap_or(var("PAGER").as_deref().unwrap_or("vi")),
            )?;
            Box::new(BufWriter::new(pager))
        } else {
            Box::new(BufWriter::new(&mut editor))
        };

        gen.write_halfdiff(&mut writer)?;
        writer
            .flush()
            .context("failed to flush the tempfile. aborting.")?;

        if args.preview {
            return Ok(());
        }
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
