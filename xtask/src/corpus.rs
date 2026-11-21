//! Pinned Salesforce corpus shared by fetch, recall and the LWC idiom suite.

use crate::recall::{clone_at, corpus_dir, PILOT_REPOS};
use serde::Deserialize;
use std::path::PathBuf;

const STAGES: &[&str] = &["v1.1", "v1.2", "v1.3"];

#[derive(Debug, Deserialize)]
pub(crate) struct Corpus {
    pub(crate) repo: Vec<Repo>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Repo {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) commit: String,
    pub(crate) stages: Vec<String>,
}

impl Repo {
    pub(crate) fn path(&self) -> PathBuf {
        if let Some((name, _, _)) = PILOT_REPOS.iter().find(|(_, url, _)| *url == self.url) {
            return corpus_dir().join("apex").join(name);
        }
        corpus_dir().join("sf").join(self.name.replace('/', "__"))
    }

    pub(crate) fn fetch(&self) -> Result<PathBuf, String> {
        let path = self.path();
        clone_at(&self.url, &self.commit, &path)?;
        Ok(path)
    }
}

impl Corpus {
    /// The metadata comparison uses the same pinned population in both harnesses.
    pub(crate) fn with_pilots(&self, stage: &str) -> Vec<Repo> {
        let mut repos: Vec<_> = self.stage(Some(stage)).cloned().collect();
        for (_, url, commit) in PILOT_REPOS {
            if !repos.iter().any(|r| r.url == url) {
                repos.push(Repo {
                    name: url.trim_start_matches("https://github.com/").into(),
                    url: url.into(),
                    commit: commit.into(),
                    stages: vec![stage.into()],
                });
            }
        }
        repos
    }

    pub(crate) fn stage<'a>(&'a self, stage: Option<&'a str>) -> impl Iterator<Item = &'a Repo> {
        self.repo
            .iter()
            .filter(move |r| stage.is_none_or(|s| r.stages.iter().any(|candidate| candidate == s)))
    }
}

fn parse(text: &str) -> Result<Corpus, String> {
    let corpus: Corpus = toml::from_str(text).map_err(|e| e.to_string())?;
    for r in &corpus.repo {
        if r.commit.len() != 40 || !r.commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("{}: commit must be a full SHA", r.name));
        }
        let parts: Vec<_> = r.name.split('/').collect();
        if parts.len() != 2
            || parts.iter().any(|p| {
                p.is_empty()
                    || *p == "."
                    || *p == ".."
                    || !p
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            })
        {
            return Err(format!("invalid owner/repository {:?}", r.name));
        }
        if r.stages.is_empty() || r.stages.iter().any(|s| !STAGES.contains(&s.as_str())) {
            return Err(format!(
                "{}: stages must contain v1.1, v1.2 or v1.3",
                r.name
            ));
        }
    }
    Ok(corpus)
}

pub(crate) fn load() -> Result<Corpus, String> {
    let path = crate::workspace_root().join("xtask/corpora/salesforce.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn parse_args(args: &[String]) -> Result<Option<&str>, String> {
    match args {
        [] => Ok(None),
        [flag, stage] if flag == "--stage" && STAGES.contains(&stage.as_str()) => Ok(Some(stage)),
        _ => Err("usage: xtask fetch-corpus [--stage v1.1|v1.2|v1.3]".into()),
    }
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let stage = parse_args(args)?;
    let corpus = load()?;
    let (mut fetched, mut skipped, mut failures) = (0, 0, Vec::new());
    for repo in corpus.stage(stage) {
        let at_pin = crate::git(Some(&repo.path()), &["rev-parse", "HEAD"])
            .is_ok_and(|head| head == repo.commit);
        match repo.fetch() {
            Ok(_) if at_pin => skipped += 1,
            Ok(_) => fetched += 1,
            Err(e) => {
                eprintln!("fetch-corpus {}: {e}", repo.name);
                failures.push(repo.name.as_str());
            }
        }
    }
    println!(
        "fetch-corpus {}: fetched {fetched}, skipped {skipped}, failed {}, total {}",
        stage.unwrap_or("all"),
        failures.len(),
        fetched + skipped + failures.len()
    );
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("failed repositories: {}", failures.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_corpus_parses_and_filters_stages() {
        let c = parse(include_str!("../corpora/salesforce.toml")).unwrap();
        assert_eq!(c.repo.len(), 73);
        assert_eq!(c.stage(None).count(), 73);
        for stage in STAGES {
            let selected: Vec<_> = c.stage(Some(stage)).collect();
            assert!(!selected.is_empty());
            assert!(selected.iter().all(|r| r.stages.iter().any(|s| s == stage)));
        }
        assert!(c
            .stage(Some("v1.1"))
            .any(|r| r.name == "trailheadapps/lwc-recipes"));
        assert!(!c
            .stage(Some("v1.1"))
            .any(|r| r.name == "SalesforceFoundation/Affiliations"));
        assert!(c.repo[0]
            .path()
            .ends_with("corpus/sf/SFDC-Assets__Get-Geolocation"));
        let invalid =
            include_str!("../corpora/salesforce.toml").replacen(&c.repo[0].commit, "abc", 1);
        assert!(parse(&invalid).unwrap_err().contains("full SHA"));
    }

    #[test]
    fn fetch_stage_arguments() {
        assert_eq!(parse_args(&[]).unwrap(), None);
        for stage in STAGES {
            let args = ["--stage".into(), (*stage).into()];
            assert_eq!(parse_args(&args).unwrap(), Some(*stage));
        }
        assert!(parse_args(&["--stage".into()]).is_err());
        assert!(parse_args(&["--stage".into(), "v2".into()]).is_err());
    }

    #[test]
    fn metadata_population_contains_only_v12_and_pilots_without_duplicates() {
        let corpus = parse(include_str!("../corpora/salesforce.toml")).unwrap();
        let repos = corpus.with_pilots("v1.2");
        assert_eq!(repos.len(), 50);
        assert!(repos
            .iter()
            .any(|r| r.name == "SalesforceFoundation/Households"));
        assert!(!repos
            .iter()
            .any(|r| r.name == "SalesforceFoundation/Affiliations"));
        assert!(!repos.iter().any(|r| r.name == "navikt/crm-hot-tolk"));
        for (name, url, commit) in PILOT_REPOS {
            let selected: Vec<_> = repos.iter().filter(|r| r.url == url).collect();
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].commit, commit);
            assert!(selected[0].path().ends_with(format!("corpus/apex/{name}")));
        }
        let mut duplicate = corpus;
        duplicate.repo.push(repos.last().unwrap().clone());
        assert_eq!(duplicate.with_pilots("v1.2").len(), 50);
    }
}
