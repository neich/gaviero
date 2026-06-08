//! Authorable skill templates for turn-scoped prompt injection.

pub mod catalog;
pub mod frontmatter;
pub mod template;

use std::path::{Path, PathBuf};

use crate::memory::scope::SCOPE_REPO;

pub use catalog::SkillCatalog;
pub use template::substitute;

/// A loaded skill markdown file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub arguments: Vec<String>,
    pub argument_hint: Option<String>,
    pub body: String,
    pub scope_level: i32,
    pub source_path: PathBuf,
}

/// A skill resolved and rendered for a single chat turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSkill {
    pub name: String,
    pub scope_level: i32,
    pub rendered_body: String,
}

/// Non-fatal warning emitted while loading or resolving skills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillWarning {
    pub name: String,
    pub message: String,
}

impl Skill {
    pub fn render(&self, args: &[String], raw_arguments: &str) -> String {
        template::substitute(&self.body, args, raw_arguments, &self.arguments)
    }
}

/// Valid skill name charset (folder name under `*/.gaviero/skills/`).
pub fn valid_stem(stem: &str) -> bool {
    !stem.is_empty()
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Skill name from a `…/<name>/SKILL.md` path.
pub fn skill_name_from_path(path: &Path) -> Option<&str> {
    if path.file_name().and_then(|f| f.to_str()) != Some("SKILL.md") {
        return None;
    }
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
}

/// Parse a skill from disk contents. `path` must be `…/<name>/SKILL.md`.
pub fn parse_skill(path: &Path, contents: &str) -> Result<Skill, SkillWarning> {
    let stem = skill_name_from_path(path).unwrap_or("?");
    let warn = |message: &str| SkillWarning {
        name: stem.to_string(),
        message: message.to_string(),
    };

    if skill_name_from_path(path).is_none() {
        return Err(warn("skill definition must be SKILL.md inside a named folder"));
    }

    if !valid_stem(stem) {
        return Err(warn("invalid skill name charset"));
    }

    let (fm_opt, body) = frontmatter::split_frontmatter(contents);
    let fm_map = fm_opt
        .map(frontmatter::parse_lines)
        .unwrap_or_default();

    let description = fm_map
        .get("description")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| warn("missing or empty description"))?;

    if let Some(declared) = fm_map.get("name") {
        if declared != stem {
            return Err(warn("frontmatter name does not match folder name"));
        }
    }

    let arguments = fm_map
        .get("arguments")
        .map(|s| frontmatter::parse_arguments(s))
        .unwrap_or_default();

    let mut seen = std::collections::HashSet::new();
    for arg in &arguments {
        if !seen.insert(arg.clone()) {
            return Err(warn("duplicate argument name in frontmatter"));
        }
    }

    let argument_hint = fm_map.get("argument-hint").cloned();

    Ok(Skill {
        name: stem.to_string(),
        description,
        arguments,
        argument_hint,
        body: body.to_string(),
        scope_level: SCOPE_REPO,
        source_path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_skill_md() -> &'static str {
        "---\n\
         description: Migrate a UI component between frameworks\n\
         arguments: [from, to]\n\
         argument-hint: <from> <to>\n\
         ---\n\
         Migrate the component from $from to $to.\n"
    }

    #[test]
    fn parse_skill_loads_frontmatter_and_body() {
        let path = Path::new("migrate-component/SKILL.md");
        let skill = parse_skill(path, sample_skill_md()).unwrap();
        assert_eq!(skill.name, "migrate-component");
        assert_eq!(skill.arguments, vec!["from", "to"]);
        assert!(skill.body.contains("Migrate the component"));
    }

    #[test]
    fn parse_skill_rejects_missing_description() {
        let path = Path::new("bad/SKILL.md");
        let src = "---\nname: bad\n---\nbody\n";
        let err = parse_skill(path, src).unwrap_err();
        assert!(err.message.contains("description"));
    }

    #[test]
    fn render_delegates_to_substitute() {
        let path = Path::new("migrate-component/SKILL.md");
        let skill = parse_skill(path, sample_skill_md()).unwrap();
        let args = vec!["React".into(), "Vue".into()];
        let rendered = skill.render(&args, "React Vue");
        assert!(rendered.contains("React"));
        assert!(rendered.contains("Vue"));
    }

}
