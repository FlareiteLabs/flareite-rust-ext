//! Rust for Flareite. Highlighting needs no code, and Flareite runs
//! rust-analyzer; this gives rust-analyzer the Cargo workspaces in the open
//! folder that it wouldn't find on its own.
//!
//! rust-analyzer loads the Cargo workspace of the open folder (or, with no
//! `Cargo.toml` there, the packages in its subfolders). A package that
//! workspace excludes is a workspace of its own, which rust-analyzer never
//! loads: an operating system's kernel built for another target, say, or
//! an example kept out of the build. Its files would get no definitions,
//! hover, or completion. So for each Rust file opened in the folder, this
//! asks Cargo which workspace the file is in (`cargo locate-project
//! --workspace`, which follows `members` and `exclude` as Cargo does), and
//! gives rust-analyzer each workspace it wouldn't load as one more
//! workspace folder (`language_servers::set_folders`).
//!
//! rust-analyzer waits for this extension to activate, so the files open
//! then are looked at right away, and rust-analyzer starts with their
//! workspaces. Files opened later are looked at in the background, and
//! rust-analyzer loads their workspaces as it runs.

mod paths;

use std::collections::{HashMap, HashSet};

use flareite_extension_api::events::Event;
use flareite_extension_api::processes::{self, ProcessError, ProcessOptions, ProcessOutput};
use flareite_extension_api::types::{Activation, DocumentInfo, LogLevel};
use flareite_extension_api::{Extension, host, language_servers, register, workspace};

const SERVER: &str = "rust-analyzer";
const CARGO: &str = "cargo";
/// Most folders looked at while activating, when rust-analyzer waits.
const MAX_AT_ACTIVATION: usize = 8;
/// How long Cargo gets to answer while rust-analyzer waits, and later.
const ACTIVATION_TIMEOUT_MS: u32 = 3_000;
const TIMEOUT_MS: u32 = 60_000;

/// The workspace rust-analyzer loads from the open folder.
#[derive(Debug, Clone, PartialEq)]
enum Base {
  /// Cargo hasn't said yet.
  Unknown,
  /// The workspace with its root in this folder (the open folder, or one
  /// it's in).
  Workspace(String),
  /// There's none: rust-analyzer loads the packages in the open folder's
  /// subfolders, each with its workspace.
  Subfolders,
}

/// What Cargo was asked.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Asked {
  /// The open folder's workspace.
  Base,
  /// The workspace of a folder with Rust files open.
  Files,
}

/// The workspace root folder in Cargo's answer: `locate-project` prints the
/// root's `Cargo.toml`.
fn workspace_root(output: &ProcessOutput) -> Option<String> {
  if output.exit_code != Some(0) {
    return None;
  }
  let stdout = String::from_utf8_lossy(&output.stdout);
  let manifest = stdout.lines().next()?.trim();
  paths::parent(manifest).map(str::to_owned)
}

#[derive(Default)]
struct Rust {
  /// The open folder, if it's trusted (rust-analyzer runs only then).
  folder: Option<String>,
  base: Option<Base>,
  /// Folders Cargo was asked about, so each is asked once.
  asked: HashSet<String>,
  /// Cargo, running in the background, by process id.
  running: HashMap<u64, Asked>,
  /// The workspaces of opened files, to look at once `base` is known.
  found: Vec<String>,
  /// The folders rust-analyzer gets.
  folders: Vec<String>,
  /// Set once Cargo can't run (it isn't installed, or isn't allowed to
  /// run), to stop asking.
  stopped: bool,
}

impl Rust {
  /// Starts over for the open folder: forgets its folders, and looks at the
  /// Rust files open in it, right away while `activating` (as far as it
  /// goes), else in the background.
  fn start(&mut self, activating: bool) {
    *self = Self::default();
    if let Err(error) = language_servers::set_folders(SERVER, &[]) {
      log_warning(&format!("couldn't set rust-analyzer's folders: {error:?}"));
    }
    let Some(folder) = workspace::folder().filter(|folder| folder.trusted) else {
      return;
    };
    self.folder = Some(folder.path.clone());
    self.base = Some(Base::Unknown);
    self.ask(Asked::Base, &folder.path, activating);
    let documents = host::documents().unwrap_or_default();
    for document in documents {
      let now = activating && self.asked.len() < MAX_AT_ACTIVATION;
      self.opened(&document, now);
    }
  }

  /// A document is open: if it's a Rust file in the open folder, asks
  /// Cargo for its workspace, once per folder.
  fn opened(&mut self, document: &DocumentInfo, now: bool) {
    let (Some(folder), Some(path)) = (&self.folder, &document.path) else {
      return;
    };
    if self.stopped || document.language.as_deref() != Some("rust") || !paths::inside(path, folder)
    {
      return;
    }
    let Some(directory) = paths::parent(path).map(str::to_owned) else {
      return;
    };
    if self.asked.insert(paths::key(&directory)) {
      self.ask(Asked::Files, &directory, now);
    }
  }

  /// Asks Cargo for the workspace of `directory`: waiting for it, `now`,
  /// else in the background.
  fn ask(&mut self, asked: Asked, directory: &str, now: bool) {
    if self.stopped {
      return;
    }
    let args = ["locate-project", "--workspace", "--message-format", "plain"].map(String::from);
    let options = ProcessOptions {
      cwd: Some(directory.to_owned()),
      env: Vec::new(),
      stdin: None,
      timeout_ms: Some(if now {
        ACTIVATION_TIMEOUT_MS
      } else {
        TIMEOUT_MS
      }),
    };
    if now {
      let outcome = processes::run(CARGO, &args, &options);
      self.answered(asked, outcome);
    } else {
      match processes::spawn(CARGO, &args, &options) {
        Ok(process) => {
          self.running.insert(process, asked);
        }
        Err(error) => self.failed(&error),
      }
    }
  }

  /// Cargo answered.
  fn answered(&mut self, asked: Asked, outcome: Result<ProcessOutput, ProcessError>) {
    let root = match outcome {
      Ok(output) => workspace_root(&output),
      Err(error) => {
        self.failed(&error);
        if asked == Asked::Base {
          // Without the open folder's workspace, nothing can be compared.
          return;
        }
        None
      }
    };
    match asked {
      Asked::Base => self.base = Some(root.map_or(Base::Subfolders, Base::Workspace)),
      Asked::Files => self.found.extend(root),
    }
    self.update();
  }

  fn failed(&mut self, error: &ProcessError) {
    match error {
      // Asked again when the folder is trusted.
      ProcessError::Untrusted => {}
      ProcessError::PermissionDenied
      | ProcessError::NotFound(_)
      | ProcessError::UnknownProgram(_) => {
        self.stopped = true;
        log_warning(&format!(
          "can't run Cargo ({error:?}), so rust-analyzer gets only the folder's own workspace"
        ));
      }
      _ => log_warning(&format!("Cargo didn't say where a workspace is: {error:?}")),
    }
  }

  /// Whether rust-analyzer loads the workspace with its root in `root`
  /// without being told: it's the open folder's, or the folder has none
  /// and it's in one of its subfolders. A workspace outside the open folder
  /// can't be given to it either.
  fn loaded(&self, root: &str) -> bool {
    let Some(folder) = &self.folder else {
      return true;
    };
    if !paths::inside(root, folder) {
      return true;
    }
    match &self.base {
      Some(Base::Workspace(base)) => paths::same(root, base),
      Some(Base::Subfolders) => {
        paths::parent(root).is_some_and(|parent| paths::same(parent, folder))
      }
      Some(Base::Unknown) | None => true,
    }
  }

  /// Gives rust-analyzer the workspaces found that it doesn't load, once
  /// the open folder's is known.
  fn update(&mut self) {
    if !matches!(self.base, Some(Base::Workspace(_) | Base::Subfolders)) {
      return;
    }
    let mut changed = false;
    for root in std::mem::take(&mut self.found) {
      let known = self.folders.iter().any(|folder| paths::same(folder, &root));
      if !known && !self.loaded(&root) {
        self.folders.push(root);
        changed = true;
      }
    }
    if changed {
      host::log(
        LogLevel::Info,
        &format!("rust-analyzer also gets {}", self.folders.join(", ")),
      );
      if let Err(error) = language_servers::set_folders(SERVER, &self.folders) {
        log_warning(&format!("couldn't set rust-analyzer's folders: {error:?}"));
      }
    }
  }
}

fn log_warning(message: &str) {
  host::log(LogLevel::Warn, message);
}

impl Extension for Rust {
  fn activate(_: Activation) -> Result<Self, String> {
    let mut rust = Rust::default();
    rust.start(true);
    Ok(rust)
  }

  fn handle_event(&mut self, event: Event) -> Result<(), String> {
    match event {
      Event::DocumentOpened(document) => self.opened(&document, false),
      Event::ProcessFinished(finished) => {
        if let Some(asked) = self.running.remove(&finished.process) {
          self.answered(asked, finished.outcome);
        }
      }
      // Another folder, or the user trusted this one.
      Event::FolderChanged => self.start(false),
      _ => {}
    }
    Ok(())
  }
}

register!(Rust);

#[cfg(test)]
mod tests {
  use super::*;

  fn rust(base: Base) -> Rust {
    Rust {
      folder: Some(r"D:\Projects\os".into()),
      base: Some(base),
      ..Rust::default()
    }
  }

  #[test]
  fn cargo_names_the_workspace_manifest() {
    let output = |code, stdout: &str| ProcessOutput {
      exit_code: Some(code),
      stdout: stdout.as_bytes().to_vec(),
      stderr: Vec::new(),
      truncated: false,
    };
    assert_eq!(
      workspace_root(&output(0, "D:\\Projects\\os\\kernel\\Cargo.toml\r\n")).as_deref(),
      Some(r"D:\Projects\os\kernel")
    );
    assert_eq!(workspace_root(&output(101, "")), None);
  }

  #[test]
  fn workspaces_outside_the_folders_one_are_given() {
    let root = rust(Base::Workspace(r"D:\Projects\os".into()));
    assert!(root.loaded(r"d:\projects\os"));
    assert!(!root.loaded(r"D:\Projects\os\kernel"));
    assert!(root.loaded(r"D:\Projects"), "outside the open folder");

    let parent = rust(Base::Workspace(r"D:\Projects".into()));
    assert!(!parent.loaded(r"D:\Projects\os\kernel"));

    // With no workspace at the folder, rust-analyzer loads its subfolders'.
    let none = rust(Base::Subfolders);
    assert!(none.loaded(r"D:\Projects\os\kernel"));
    assert!(!none.loaded(r"D:\Projects\os\crates\kernel"));
  }

  #[test]
  fn nothing_is_given_until_the_folders_workspace_is_known() {
    let mut waiting = rust(Base::Unknown);
    waiting.found.push(r"D:\Projects\os\kernel".into());
    waiting.update();
    assert!(waiting.folders.is_empty());
    assert_eq!(waiting.found.len(), 1);
  }
}
