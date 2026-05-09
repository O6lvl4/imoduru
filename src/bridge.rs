use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Serialize)]
struct Request {
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<FetchParams>,
}

#[derive(Debug, Serialize)]
pub struct FetchParams {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_for: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct Response {
    #[allow(dead_code)]
    pub id: Option<u64>,
    pub ok: bool,
    pub html: Option<String>,
    #[allow(dead_code)]
    pub url: Option<String>,
    pub status: Option<u16>,
    pub error: Option<String>,
    #[serde(default)]
    pub ready: bool,
}

pub struct Playwright {
    child: Child,
    stdin: std::process::ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    pub timeout: u64,
}

impl Playwright {
    pub fn spawn(timeout: u64) -> Result<Self> {
        let bridge_dir = Self::bridge_dir()?;

        let mut child = Command::new("node")
            .arg("index.mjs")
            .current_dir(&bridge_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("failed to spawn playwright bridge (is node installed?)")?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut pw = Playwright {
            child,
            stdin,
            reader: BufReader::new(stdout),
            timeout,
        };

        let resp = pw.read_response()?;
        if !resp.ready {
            bail!("bridge did not send ready signal");
        }

        Ok(pw)
    }

    fn bridge_dir() -> Result<String> {
        let exe = std::env::current_exe()?;
        let exe_dir = exe.parent().unwrap();

        for ancestor in exe_dir.ancestors() {
            let candidate = ancestor.join("bridge");
            if candidate.join("index.mjs").exists() {
                return Ok(candidate.to_string_lossy().into_owned());
            }
        }

        bail!("cannot find bridge/index.mjs — run from the imoduru project directory");
    }

    fn send_request(&mut self, req: &Request) -> Result<()> {
        let line = serde_json::to_string(req)?;
        writeln!(self.stdin, "{line}")?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read_response(&mut self) -> Result<Response> {
        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        let resp: Response =
            serde_json::from_str(&line).context("failed to parse bridge response")?;
        Ok(resp)
    }

    pub fn fetch(&mut self, url: &str) -> Result<Response> {
        self.fetch_with(url, None, None)
    }

    pub fn fetch_with(
        &mut self,
        url: &str,
        wait_for: Option<&str>,
        delay: Option<u64>,
    ) -> Result<Response> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let req = Request {
            id,
            method: "fetch".into(),
            params: Some(FetchParams {
                url: url.to_string(),
                wait_until: None,
                wait_for: wait_for.map(|s| s.to_string()),
                delay,
                timeout: Some(self.timeout),
            }),
        };
        self.send_request(&req)?;
        let resp = self.read_response()?;
        if !resp.ok {
            bail!(
                "fetch failed for {}: {}",
                url,
                resp.error.as_deref().unwrap_or("unknown error")
            );
        }
        Ok(resp)
    }

    pub fn shutdown(&mut self) -> Result<()> {
        let req = Request {
            id: 0,
            method: "shutdown".into(),
            params: None,
        };
        let _ = self.send_request(&req);
        let _ = self.child.wait();
        Ok(())
    }
}
