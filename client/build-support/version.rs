// The version the client binaries report: the one the package they ship in
// carries (dev/version.sh). Included by the build scripts that emit it as
// MYNA_VERSION.

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Resolved {
    pub version: String,
    // Every path exists: cargo reruns a build script on every build while a
    // watched path is missing.
    pub watch: Vec<PathBuf>,
}

// `workspace` is the Cargo workspace root, `checkout` the repository it came
// from. The Cargo version is a placeholder, so there is no fallback.
pub fn resolve(workspace: &Path, checkout: &Path) -> Result<Resolved, String> {
    // A snap build instance has no .git, so the packaging stages the version.
    let staged = workspace.join(".version");
    if let Ok(version) = std::fs::read_to_string(&staged) {
        return Ok(Resolved {
            version: version.trim().to_owned(),
            watch: vec![staged],
        });
    }
    from_checkout(checkout).ok_or_else(|| {
        format!(
            "no version: {} is not staged and {} is not a git checkout with dev/version.sh",
            staged.display(),
            checkout.display()
        )
    })
}

// Only HEAD and the refs are watched, so -dirty is never reported: watching
// the working tree would rebuild on every edit.
fn from_checkout(checkout: &Path) -> Option<Resolved> {
    let script = checkout.join("dev/version.sh");
    let version = stdout(&mut Command::new(&script))?;
    let mut watch = vec![script];
    for name in ["HEAD", "packed-refs", "refs/heads", "refs/tags"] {
        let path = PathBuf::from(stdout(Command::new("git").arg("-C").arg(checkout).args([
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            name,
        ]))?);
        if path.exists() {
            watch.push(path);
        }
    }
    Some(Resolved { version, watch })
}

fn stdout(command: &mut Command) -> Option<String> {
    let output = command.output().ok().filter(|o| o.status.success())?;
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

#[allow(dead_code)] // Tests include this file for `resolve` alone.
pub fn emit() {
    let manifest =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let workspace = manifest.join("..");
    // cargo-mutants builds a copy of client/ alone; MYNA_REPO_ROOT names the
    // checkout it was copied from.
    println!("cargo:rerun-if-env-changed=MYNA_REPO_ROOT");
    let checkout = std::env::var_os("MYNA_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join(".."));
    let resolved = resolve(&workspace, &checkout).unwrap_or_else(|error| panic!("{error}"));
    for path in &resolved.watch {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rustc-env=MYNA_VERSION={}", resolved.version);
}
