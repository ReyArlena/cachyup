//! Build the ordered list of steps from the configuration. Same order as the
//! bash version: checks, system update, prerequisites, apps, tweaks, cleanup,
//! post-update checks.

use crate::catalog::{Catalog, Source};
use crate::config::Config;
use crate::selection::Selection;

use super::steps;
use super::{Ctx, Outcome};

pub type Body = Box<dyn FnOnce(&mut Ctx) -> Outcome + Send>;

pub struct Step {
    pub group: String,
    pub name: String,
    /// A failure stops the run (preflight checks).
    pub required: bool,
    /// Runs, but is not shown or counted (starting background work).
    pub hidden: bool,
    pub body: Body,
}

impl Step {
    pub fn new(
        group: &str,
        name: impl Into<String>,
        body: impl FnOnce(&mut Ctx) -> Outcome + Send + 'static,
    ) -> Self {
        Self {
            group: group.into(),
            name: name.into(),
            required: false,
            hidden: false,
            body: Box::new(body),
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }
}

pub struct Plan {
    pub steps: Vec<Step>,
    /// Task labels switched on, for the record.
    pub tasks: Vec<String>,
}

impl Plan {
    /// Number of visible steps per group, in order. For summaries.
    pub fn visible(&self) -> impl Iterator<Item = &Step> {
        self.steps.iter().filter(|s| !s.hidden)
    }
}

pub const GROUP_PREFLIGHT: &str = "Preflight";
pub const GROUP_UPDATE: &str = "System update";
pub const GROUP_PREREQ: &str = "Prerequisites";
pub const GROUP_TWEAKS: &str = "Tweaks";
pub const GROUP_CLEANUP: &str = "Cleanup";
pub const GROUP_CHECKS: &str = "Post-update checks";

pub const TASK_UPDATE: &str = "Full System Update";
pub const TASK_APPS: &str = "Install Apps";
pub const TASK_TWEAKS: &str = "Install Tweaks";

pub fn build_plan(catalog: &Catalog, config: &Config, selection: &Selection, dry: bool) -> Plan {
    let mut plan = Plan {
        steps: Vec::new(),
        tasks: Vec::new(),
    };
    let tasks = &config.tasks;
    if tasks.update {
        plan.tasks.push(TASK_UPDATE.into());
    }
    if tasks.apps {
        plan.tasks.push(TASK_APPS.into());
    }
    if tasks.tweaks {
        plan.tasks.push(TASK_TWEAKS.into());
    }
    if plan.tasks.is_empty() {
        return plan;
    }
    let s = &mut plan.steps;

    steps::preflight(s, dry);

    if tasks.update {
        steps::system_update(s, config);
    }

    steps::prerequisites(s, &catalog.prerequisites);

    if tasks.apps {
        for source in Source::ALL {
            let apps: Vec<_> = catalog
                .apps_from(source)
                .filter(|a| selection.app_on(&a.key()))
                .cloned()
                .collect();
            if !apps.is_empty() {
                steps::apps(s, source, apps);
            }
        }
    }

    if tasks.tweaks {
        let tweaks: Vec<_> = catalog
            .tweaks
            .iter()
            .filter(|t| selection.tweak_on(&t.id))
            .map(|t| {
                let app = t.app.as_ref().and_then(|k| catalog.app(k)).cloned();
                // Will this run install the app first? Matters for dry runs.
                let app_queued = tasks.apps && t.app.as_ref().is_some_and(|k| selection.app_on(k));
                (t.clone(), app, app_queued)
            })
            .collect();
        if !tweaks.is_empty() {
            steps::tweaks(s, tweaks);
        }
    }

    if tasks.update {
        steps::cleanup(s, config);
        steps::post_checks(s);
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(plan: &Plan) -> Vec<String> {
        plan.visible()
            .map(|s| format!("{}: {}", s.group, s.name))
            .collect()
    }

    #[test]
    fn nothing_selected_means_no_steps() {
        let mut config = Config::default();
        config.tasks.update = false;
        config.tasks.apps = false;
        config.tasks.tweaks = false;
        let plan = build_plan(&Catalog::builtin(), &config, &Selection::default(), true);
        assert!(plan.steps.is_empty());
    }

    #[test]
    fn plan_follows_tasks_and_selection() {
        let catalog = Catalog::builtin();
        let mut sel = Selection::default();
        sel.apps.insert("pacman/discord".into(), true);
        sel.apps.insert("aur/spotify".into(), true);
        sel.apps.insert("pacman/ark".into(), false);
        sel.tweaks.insert("discord-skip-host-update".into(), true);
        let mut config = Config::default();
        config.update.insert("pacman".into(), false);
        let plan = build_plan(&catalog, &config, &sel, true);
        let n = names(&plan);
        assert!(n.contains(&"Pacman packages: discord".to_string()), "{n:?}");
        assert!(n.contains(&"AUR packages: spotify".to_string()));
        assert!(!n.iter().any(|s| s.ends_with(": ark")));
        assert!(!n.iter().any(|s| s.contains("pacman -Syu")));
        assert!(
            !n.iter().any(|s| s.contains("Keyrings")),
            "keyrings need -Syu"
        );
        assert!(n.contains(&"Tweaks: discord: Disable host update".to_string()));
        assert!(n.iter().any(|s| s.starts_with("Cleanup:")));
        let first_prereq = n
            .iter()
            .position(|s| s.starts_with("Prerequisites"))
            .unwrap();
        let first_app = n
            .iter()
            .position(|s| s.starts_with("Pacman packages"))
            .unwrap();
        assert!(first_prereq < first_app);

        config.tasks.update = false;
        let plan = build_plan(&catalog, &config, &sel, true);
        let n = names(&plan);
        assert!(!n.iter().any(|s| s.starts_with("Cleanup:")));
        assert!(n.iter().any(|s| s.starts_with("Prerequisites")));
    }
}
