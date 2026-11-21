//! Developer tasks: `cargo run -p xtask -- <command> [args]`.
//!
//! - `fetch-reference [DIR]`  clone facebookresearch/aroma-paper-artifacts at the pinned
//!   commit into DIR (default `<workspace>/ref/aroma-paper-artifacts`). The repository is
//!   CC BY-NC-SA 4.0: it is fetched for tests, never copied into this repo.
//! - `fetch-corpus [--stage v1.1|v1.2|v1.3]` fetch pinned Salesforce repositories.
//! - `gen-goldens`, `recall`, `idiom`, `perf`  see their modules.
mod corpus;
mod gen_goldens;
mod idiom;
mod perf;
mod recall;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const REFERENCE_URL: &str = "https://github.com/facebookresearch/aroma-paper-artifacts";
const REFERENCE_COMMIT: &str = "e8d8beeb99255d8929874c156fd65aac74d21508";

const USAGE: &str = "usage: cargo run -p xtask -- <fetch-reference [DIR] | fetch-corpus [--stage v1.1|v1.2|v1.3] | gen-goldens | recall | idiom | perf> [args]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((cmd, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let result = match cmd.as_str() {
        "fetch-reference" => fetch_reference(rest),
        "fetch-corpus" => corpus::run(rest),
        "gen-goldens" => gen_goldens::run(rest),
        "recall" => recall::run(rest),
        "idiom" => idiom::run(rest),
        "perf" => perf::run(rest),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command {other:?}\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask {cmd}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root")
        .to_path_buf()
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    let out = cmd
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn fetch_reference(args: &[String]) -> Result<(), String> {
    let dest = match args {
        [] => workspace_root().join("ref").join("aroma-paper-artifacts"),
        [dir] => PathBuf::from(dir),
        _ => return Err(format!("expected at most one directory argument\n{USAGE}")),
    };
    if dest.join(".git").exists() {
        if git(Some(&dest), &["rev-parse", "HEAD"])? == REFERENCE_COMMIT {
            println!(
                "reference artifacts already at {REFERENCE_COMMIT} in {}",
                dest.display()
            );
            return Ok(());
        }
        if git(
            Some(&dest),
            &["cat-file", "-e", &format!("{REFERENCE_COMMIT}^{{commit}}")],
        )
        .is_err()
        {
            git(Some(&dest), &["fetch", "--quiet", "origin"])?;
        }
    } else if dest.exists() {
        return Err(format!(
            "{} exists but is not a git checkout; remove it first",
            dest.display()
        ));
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        let dest_str = dest.to_str().ok_or("destination path is not UTF-8")?;
        git(None, &["clone", "--quiet", REFERENCE_URL, dest_str])?;
    }
    git(
        Some(&dest),
        &["checkout", "--quiet", "--detach", REFERENCE_COMMIT],
    )?;
    let head = git(Some(&dest), &["rev-parse", "HEAD"])?;
    if head != REFERENCE_COMMIT {
        return Err(format!("checked out {head}, expected {REFERENCE_COMMIT}"));
    }
    println!(
        "reference artifacts at {REFERENCE_COMMIT} in {}",
        dest.display()
    );
    Ok(())
}
