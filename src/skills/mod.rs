pub struct BuiltinSkill {
    pub name: &'static str,
    pub description: &'static str,
    pub content: &'static str,
    pub filename: &'static str,
}

pub const SKILLS: &[BuiltinSkill] = &[BuiltinSkill {
    name: "agentsight-diagnose",
    description: "Diagnose session efficiency and suggest CLAUDE.md improvements",
    content: include_str!("agentsight_diagnose.md"),
    filename: "agentsight-diagnose.md",
}];

pub fn find_skill(name: &str) -> Option<&'static BuiltinSkill> {
    SKILLS.iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_skill_exists() {
        let skill = find_skill("agentsight-diagnose");
        assert!(skill.is_some());
        let skill = skill.unwrap();
        assert_eq!(skill.name, "agentsight-diagnose");
        assert_eq!(skill.filename, "agentsight-diagnose.md");
    }

    #[test]
    fn test_find_skill_missing() {
        assert!(find_skill("nonexistent").is_none());
    }

    #[test]
    fn test_skill_content_has_frontmatter() {
        let skill = find_skill("agentsight-diagnose").unwrap();
        assert!(
            skill.content.starts_with("---"),
            "Skill content should start with YAML frontmatter"
        );
    }

    #[test]
    fn test_skill_content_has_description() {
        let skill = find_skill("agentsight-diagnose").unwrap();
        assert!(
            skill.content.contains("description:"),
            "Skill content should contain a description field in frontmatter"
        );
    }
}
