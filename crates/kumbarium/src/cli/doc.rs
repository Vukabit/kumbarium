//! `kum doc` (D-054): the library as a static site, cargo-doc
//! style. site.rs renders; this writes the build beside its
//! destination, swaps it in wholesale, and finishes the export
//! spine's flags (--open goes to the browser: an HTML artifact
//! opens with the OS opener, D-054's amendment to D-031).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::super::{open_stores, site};
use super::term::*;

struct DocOpts {
  site: site::Options,
  out: Option<String>,
  show: bool,
  open: bool,
}

fn parse(rest: &[&str]) -> Result<DocOpts, String> {
  let mut opts = DocOpts {
    site: site::Options {
      scope: None,
      all: false,
    },
    out: None,
    show: false,
    open: false,
  };
  let mut it = rest.iter();
  while let Some(arg) = it.next() {
    match *arg {
      "--all" => opts.site.all = true,
      "--show" => opts.show = true,
      "--open" => opts.open = true,
      "--out" => match it.next() {
        Some(dir) => opts.out = Some((*dir).to_string()),
        None => return Err("--out needs a directory".into()),
      },
      "--stdout" => {
        return Err(
          "a doc build is a directory, not a stream; --out DIR \
                    puts it elsewhere"
            .into(),
        );
      }
      flag if flag.starts_with('-') => {
        return Err(format!(
          "unknown doc flag {flag:?}; usage: kumbarium doc [ns] [--all] \
           [--out DIR] [--show] [--open]"
        ));
      }
      scope => {
        if opts.site.scope.is_some() {
          return Err("doc takes one scope; its subtree comes with it".into());
        }
        opts.site.scope = Some(scope.to_string());
      }
    }
  }
  Ok(opts)
}

pub(crate) fn doc_cmd(rest: &[&str]) -> ExitCode {
  let opts = match parse(rest) {
    Ok(o) => o,
    Err(e) => return fail(&e),
  };
  let (p, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let built = match site::build(&mut state, &opts.site) {
    Ok(s) => s,
    Err(e) => return fail(&e),
  };
  let dir = match &opts.out {
    Some(raw) => expand_home(raw),
    None => p.exports_dir.join("doc"),
  };
  if let Err(e) = install(&dir, &built.files) {
    return fail(&e);
  }
  let index = dir.join("index.html");
  let sty = super::super::style::Style::detect();
  eprintln!(
    "{}",
    sty.dim(&format!(
      "documented {} {}, {} {}",
      built.shelves,
      if built.shelves == 1 {
        "shelf"
      } else {
        "shelves"
      },
      built.facts,
      if built.facts == 1 { "fact" } else { "facts" },
    ))
  );
  println!("{}", shell_quote(&index.display().to_string()));
  if opts.show
    && let Err(e) = reveal(&index)
  {
    return fail(&e);
  }
  if opts.open
    && let Err(e) = open_artifact(&index)
  {
    return fail(&e);
  }
  ExitCode::SUCCESS
}

/// Write the build into a sibling directory, then swap it in.
/// An existing destination is replaced only when it is empty or
/// a previous doc build (carries the marker): `--out ~/Documents`
/// must never delete someone's folder.
fn install(
  dir: &Path,
  files: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
  if dir.exists() {
    let is_build = dir.join(site::MARKER).is_file();
    let is_empty = std::fs::read_dir(dir)
      .map(|mut d| d.next().is_none())
      .unwrap_or(false);
    if !is_build && !is_empty {
      return Err(format!(
        "{} exists and is not a kum doc build; refusing to replace it \
         (pick an empty or new --out)",
        dir.display()
      ));
    }
  }
  let parent = dir.parent().unwrap_or(Path::new("."));
  std::fs::create_dir_all(parent)
    .map_err(|e| format!("creating {}: {e}", parent.display()))?;
  let name = dir
    .file_name()
    .map(|n| n.to_string_lossy().into_owned())
    .unwrap_or_else(|| "doc".into());
  let pid = std::process::id();
  let staging = parent.join(format!(".{name}.building-{pid}"));
  let retired = parent.join(format!(".{name}.old-{pid}"));
  let _ = std::fs::remove_dir_all(&staging);
  let result = write_tree(&staging, files).and_then(|()| {
    if dir.exists() {
      std::fs::rename(dir, &retired)
        .map_err(|e| format!("moving the old build aside: {e}"))?;
    }
    std::fs::rename(&staging, dir).map_err(|e| {
      // Put the old build back rather than leave nothing.
      let _ = std::fs::rename(&retired, dir);
      format!("swapping in the new build: {e}")
    })
  });
  let _ = std::fs::remove_dir_all(&staging);
  let _ = std::fs::remove_dir_all(&retired);
  result
}

fn write_tree(
  root: &Path,
  files: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
  for (rel, content) in files {
    let target: PathBuf = root.join(rel);
    if let Some(dir) = target.parent() {
      std::fs::create_dir_all(dir)
        .map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    std::fs::write(&target, content)
      .map_err(|e| format!("writing {}: {e}", target.display()))?;
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parse_takes_one_scope_and_the_spine_flags() {
    let o = parse(&["project", "--all", "--out", "/tmp/x", "--open"]).unwrap();
    assert_eq!(o.site.scope.as_deref(), Some("project"));
    assert!(o.site.all && o.open && !o.show);
    assert_eq!(o.out.as_deref(), Some("/tmp/x"));
    assert!(parse(&["a", "b"]).is_err());
    assert!(parse(&["--stdout"]).is_err());
    assert!(parse(&["--bogus"]).is_err());
    assert!(parse(&[]).unwrap().site.scope.is_none());
  }

  #[test]
  fn install_replaces_a_build_but_never_a_foreign_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("doc");
    let mut files = std::collections::BTreeMap::new();
    files.insert(site::MARKER.to_string(), "m".to_string());
    files.insert("index.html".to_string(), "one".to_string());
    install(&dir, &files).unwrap();
    files.insert("index.html".to_string(), "two".to_string());
    install(&dir, &files).unwrap();
    assert_eq!(
      std::fs::read_to_string(dir.join("index.html")).unwrap(),
      "two"
    );

    let foreign = tmp.path().join("mine");
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(foreign.join("precious.txt"), "keep").unwrap();
    assert!(install(&foreign, &files).is_err());
    assert!(foreign.join("precious.txt").exists());
    // Nothing left behind beside the destinations.
    let debris: Vec<_> = std::fs::read_dir(tmp.path())
      .unwrap()
      .filter_map(|e| e.ok())
      .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
      .collect();
    assert!(debris.is_empty());
  }
}
