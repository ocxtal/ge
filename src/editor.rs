use anyhow::{anyhow, Context, Result};
use std::io::{Read, Seek, SeekFrom, Write};
use std::process::Command;
use tempfile::NamedTempFile;

pub struct Editor {
    args: Vec<String>,
    file: NamedTempFile,
}

impl Editor {
    pub fn new(editor: &str) -> Result<Self> {
        // create tempfile first
        let file = NamedTempFile::new().context("failed to create tempfile. aborting.")?;
        let name = file.path().to_str().unwrap().to_string();

        // break it by spaces to extract the base command
        let mut args: Vec<_> = editor.split_whitespace().map(|x| x.to_string()).collect();

        // check if it exists
        if !Self::exists(&args[0]) {
            return Err(anyhow!(
                "failed to find the editor {:?} in the PATH. aborting.",
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

    pub fn wait_edit(&mut self) -> Result<()> {
        // invoke the actual process here
        let mut editor = Command::new(&self.args[0])
            .args(&self.args[1..])
            .spawn()
            .with_context(|| format!("failed to start the editor: {}. aborting.", self.args[0]))?;
        let output = editor
            .wait()
            .context("editor exited unexpectedly. aborting.")?;
        if !output.success() {
            return Err(anyhow!("editor exited and returned an error. aborting."));
        }

        // make sure the tempfile exists
        // (vim and some other editors creates another working file and rename it to the original on quit,
        // which cause a missing-tempfile error. so here we check the tempfile we created still exists
        // with the same inode, by re-opening the file after the editor finished.)
        let _file = self.file.reopen().context(
            "the tempfile is missing (the editor might have closed or changed the inode of the file). aborting.",
        )?;

        // seek to the head before reading the content...
        self.file
            .seek(SeekFrom::Start(0))
            .context("failed to seek the tempfile. aborting.")?;

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
