use anyhow::{bail, Result};
use std::fs;
use std::path::PathBuf;

use crate::skills;

#[allow(dead_code)]
pub struct InstallSkillArgs {
    pub name: Option<String>,
    pub list: bool,
    pub force: bool,
    pub json: bool,
    pub verbose: bool,
}

pub fn run(args: &InstallSkillArgs) -> Result<()> {
    if args.list {
        return list_skills(args.json);
    }

    let dest_dir = commands_dir()?;
    fs::create_dir_all(&dest_dir)?;

    match &args.name {
        Some(name) => {
            let skill = skills::find_skill(name)
                .ok_or_else(|| anyhow::anyhow!("Unknown skill '{}'. Use --list to see available skills.", name))?;
            install_one(skill, &dest_dir, args.force, args.json)?;
        }
        None => {
            for skill in skills::SKILLS {
                install_one(skill, &dest_dir, args.force, args.json)?;
            }
        }
    }

    Ok(())
}

fn commands_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".claude").join("commands"))
}

fn install_one(
    skill: &skills::BuiltinSkill,
    dest_dir: &std::path::Path,
    force: bool,
    json: bool,
) -> Result<()> {
    let dest = dest_dir.join(skill.filename);

    if dest.exists() && !force {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "skill": skill.name,
                    "status": "skipped",
                    "reason": "already exists",
                    "path": dest.display().to_string(),
                    "hint": "Use --force to overwrite"
                })
            );
        } else {
            eprintln!(
                "Skipped /{}  (already exists at {}). Use --force to overwrite.",
                skill.name,
                dest.display()
            );
        }
        return Ok(());
    }

    fs::write(&dest, skill.content)?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "skill": skill.name,
                "status": "installed",
                "path": dest.display().to_string()
            })
        );
    } else {
        println!(
            "Installed /{}  ->  {}",
            skill.name,
            dest.display()
        );
    }

    Ok(())
}

fn list_skills(json: bool) -> Result<()> {
    if skills::SKILLS.is_empty() {
        bail!("No built-in skills available");
    }

    if json {
        let items: Vec<_> = skills::SKILLS
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "filename": s.filename,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else {
        println!("Available skills:");
        println!();
        for skill in skills::SKILLS {
            println!("  /{}  — {}", skill.name, skill.description);
        }
        println!();
        println!("Run `agentsight install-skill` to install all, or `agentsight install-skill <name>` for one.");
    }

    Ok(())
}
