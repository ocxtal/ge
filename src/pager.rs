use anyhow::{anyhow, Context, Result};
use atty::Stream;
use std::io::Write;
use std::process::{Child, Command, Stdio};

pub struct Pager {
    #[allow(dead_code)]
    pager: Option<Child>,
    drain: Box<dyn Write>,
}

impl Pager {
    pub fn new(pager: &str) -> Result<Self> {
        // bypass pager if piped to another command
        if !atty::is(Stream::Stdout) {
            return Ok(Pager {
                pager: None,
                drain: Box::new(std::io::stdout()),
            });
        }

        let args: Vec<_> = pager.split_whitespace().map(|x| x.to_string()).collect();
        let mut child = Command::new(&args[0])
            .args(&args[1..])
            .stdin(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to start pager {:?}. aborting.", args[0]))?;

        let stdin = child.stdin.take().unwrap();
        Ok(Pager {
            pager: Some(child),
            drain: Box::new(stdin),
        })
    }

    pub fn wait(&mut self) -> Result<()> {
        if self.pager.is_none() {
            return Ok(());
        }

        let mut pager = self.pager.take().unwrap();
        let output = pager
            .wait()
            .context("pager exited unexpectedly. aborting.")?;
        if !output.success() {
            return Err(anyhow!("pager exited and returned an error. aborting."));
        }
        Ok(())
    }
}

impl Write for Pager {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.drain.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.drain.flush()
    }
}
