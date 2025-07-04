use anyhow::{Context, Result, anyhow};
use std::io::{Read, Seek, Write};
use std::process::{Command, Stdio};
use tempfile::NamedTempFile;

pub struct Editor {
    args: Vec<String>,
    file: NamedTempFile,
    read_stdout: bool,
    buf: Vec<u8>,
}

impl Editor {
    pub fn new(editor: &str, read_stdout: bool) -> Result<Self> {
        // create tempfile first
        let file = NamedTempFile::new().context("failed to create tempfile. aborting.")?;
        let name = file.path().to_str().unwrap().to_string();

        // break it by spaces to extract the base command
        let mut args: Vec<_> =
            shlex::split(editor).context("failed to parse the editor command. aborting.")?;

        // check if it exists
        if !Self::exists(&args[0]) {
            return Err(anyhow!(
                "failed to find editor {:?} in the PATH. aborting.",
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

        Ok(Editor {
            args,
            file,
            read_stdout,
            buf: Vec::new(),
        })
    }

    fn exists(editor: &str) -> bool {
        let output = Command::new("/bin/sh")
            .args(["-c", &format!("command -v {editor}")])
            .output();
        if output.is_err() {
            return false;
        }

        output.unwrap().status.success()
    }

    fn is_a_vim(editor: &str) -> bool {
        if editor.starts_with("nano") {
            // nano doesn't recognize "--version" nor "--help"
            return false;
        }

        let output = Command::new(editor).args(["--version"]).output();
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

    pub fn wait(&mut self) -> Result<()> {
        // invoke the actual process here
        let editor = Command::new(&self.args[0])
            .args(&self.args[1..])
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to start editor {:?}. aborting.", self.args[0]))?;
        let output = editor
            .wait_with_output()
            .context("editor exited unexpectedly. aborting.")?;
        if !output.status.success() {
            return Err(anyhow!("editor exited and returned an error. aborting."));
        }

        if self.read_stdout {
            self.buf.extend_from_slice(&output.stdout);
        } else {
            // make sure the tempfile exists
            // (vim and some other editors creates another working file and rename it to the original on quit,
            // which cause a missing-tempfile error. so here we check the tempfile we created still exists
            // with the same inode, by re-opening the file after the editor finished.)
            let _file = self.file.reopen().context(
                "the tempfile is missing (the editor might have closed or changed the inode of the file). aborting.",
            )?;

            // seek to the head before reading the content...
            self.file
                .rewind()
                .context("failed to seek the tempfile. aborting.")?;
            self.file
                .read_to_end(&mut self.buf)
                .context("failed to read the tempfile. aborting.")?;
        }
        Ok(())
    }

    pub fn get_buf(&self) -> &[u8] {
        &self.buf
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

#[cfg(test)]
mod tests {
    use crate::Editor;
    use std::io::Write;

    // we expect nano, vim, and grep exist in the environment
    #[test]
    fn test_exists() {
        assert!(Editor::exists("nano"));
        assert!(Editor::exists("vim"));
        assert!(Editor::exists("grep"));

        assert!(!Editor::exists("this-is-not-a-command"));
        assert!(!Editor::exists("abcabcabcabcd"));
    }

    #[test]
    fn test_is_a_vim() {
        assert!(!Editor::is_a_vim("nano"));
        assert!(Editor::is_a_vim("vim"));
        assert!(!Editor::is_a_vim("grep"));

        assert!(!Editor::is_a_vim("this-is-not-a-command"));
        assert!(!Editor::is_a_vim("abcabcabcabcd"));
    }

    #[test]
    fn test_passthrough() {
        let mut editor = Editor::new("touch", false).unwrap();

        let input = "the quick brown fox jumps over the lazy dog.";
        editor.write_all(input.as_bytes()).unwrap();
        editor.wait().unwrap();
        assert_eq!(editor.get_buf(), input.as_bytes());
    }
}
