//! Paths as the host gives them, which may be Windows paths: WebAssembly's
//! `Path` splits only on `/`, so these work on the text, with `/` and `\`
//! both separators, and Windows paths compared ignoring case.

fn is_separator(c: char) -> bool {
  c == '/' || c == '\\'
}

/// `path` with `/` separators, no trailing separator, and lowercase if it's
/// a Windows path (which ignore case), for comparing.
pub fn key(path: &str) -> String {
  let windows = path.contains('\\') || path.as_bytes().get(1) == Some(&b':');
  let path = path.replace('\\', "/");
  let path = path.trim_end_matches('/');
  if windows {
    path.to_lowercase()
  } else {
    path.to_owned()
  }
}

/// Whether two paths name the same file or folder.
pub fn same(a: &str, b: &str) -> bool {
  key(a) == key(b)
}

/// Whether `path` is inside `folder` (and isn't the folder itself).
pub fn inside(path: &str, folder: &str) -> bool {
  let (path, folder) = (key(path), key(folder));
  path
    .strip_prefix(&folder)
    .is_some_and(|rest| rest.starts_with('/') && rest.len() > 1)
}

/// The folder `path` is in.
pub fn parent(path: &str) -> Option<&str> {
  let path = path.trim_end_matches(is_separator);
  let end = path.rfind(is_separator)?;
  let parent = &path[..end];
  // A drive's root keeps its separator (`C:\`), as does Unix's (`/`).
  if parent.is_empty() || parent.ends_with(':') {
    Some(&path[..=end])
  } else {
    Some(parent)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn windows_paths_ignore_case_and_separators() {
    assert!(same(r"D:\Projects\os\", "d:/projects/OS"));
    assert!(!same("/home/a/OS", "/home/a/os"));
    assert!(inside(
      r"D:\Projects\os\kernel\src\main.rs",
      r"d:\projects\os"
    ));
    assert!(inside("/home/a/os/kernel", "/home/a/os/"));
    assert!(!inside(r"D:\Projects\os", r"D:\Projects\os"));
    assert!(!inside(r"D:\Projects\os-tools\a.rs", r"D:\Projects\os"));
    assert!(!inside("/home/a/other", "/home/a/os"));
  }

  #[test]
  fn parents() {
    assert_eq!(
      parent(r"D:\Projects\os\kernel\Cargo.toml"),
      Some(r"D:\Projects\os\kernel")
    );
    assert_eq!(parent("/home/a/os/src/"), Some("/home/a/os"));
    assert_eq!(parent(r"C:\main.rs"), Some(r"C:\"));
    assert_eq!(parent("/main.rs"), Some("/"));
    assert_eq!(parent("main.rs"), None);
  }
}
