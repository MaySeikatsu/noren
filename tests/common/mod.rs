//! Shared e2e harness: isolated HOME + stubbed binaries on PATH.
#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

pub const BIN: &str = env!("CARGO_BIN_EXE_noren");

/// Isolated HOME + stub-bin dir prepended to PATH.
pub struct Env {
    pub _tmp: tempfile::TempDir,
    pub home: std::path::PathBuf,
    pub bin: std::path::PathBuf,
}

impl Env {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        let bin = tmp.path().join("bin");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        Env {
            home,
            bin,
            _tmp: tmp,
        }
    }

    pub fn stub(&self, name: &str, script: &str) {
        let p = self.bin.join(name);
        fs::write(&p, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }

    pub fn run(&self, args: &[&str]) -> (String, String, bool) {
        self.run_env(args, &[])
    }

    pub fn run_env(&self, args: &[&str], extra_env: &[(&str, &str)]) -> (String, String, bool) {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new(BIN);
        cmd.args(args)
            .env("HOME", &self.home)
            .env("PATH", path)
            .env_remove("ZELLIJ")
            .env_remove("ZELLIJ_SESSION_NAME");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run noren");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.success(),
        )
    }

    /// Stub zellij that appends its argv to $HOME/zellij-args.log.
    pub fn stub_zellij_logging(&self) {
        self.stub("zellij", "echo \"$@\" >> \"$HOME/zellij-args.log\"\nexit 0");
    }

    pub fn zellij_log(&self) -> Vec<String> {
        fs::read_to_string(self.home.join("zellij-args.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}
