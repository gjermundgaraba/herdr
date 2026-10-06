//! Plugin entries offered on space, tab, and pane context menus: every action
//! and pane of an enabled plugin that declares a menu context and runs on this
//! platform. The client lists the entries whose contexts match what was
//! clicked, and invokes them with that target.

use super::{effective_platforms, platform_supported, plugin_manifest_available};
use crate::api::schema::{PluginActionContext, PluginPanePlacement};
use crate::app::App;
use crate::protocol::ClientShellPluginEntry;

impl App {
    pub(crate) fn client_shell_plugin_entries(&self) -> Vec<ClientShellPluginEntry> {
        let menu_contexts = |contexts: &[PluginActionContext]| {
            contexts
                .iter()
                .copied()
                .filter(|context| *context != PluginActionContext::Global)
                .collect::<Vec<_>>()
        };
        let mut entries = Vec::new();
        for plugin in self
            .state
            .installed_plugins
            .values()
            .filter(|plugin| plugin.enabled && plugin_manifest_available(plugin))
        {
            let runs_here =
                |platforms| platform_supported(effective_platforms(platforms, &plugin.platforms));
            let entry = |entry_id: &str, title: &str, contexts, popup| ClientShellPluginEntry {
                plugin_id: plugin.plugin_id.clone(),
                entry_id: entry_id.to_owned(),
                title: title.to_owned(),
                contexts,
                popup,
            };
            entries.extend(
                plugin
                    .actions
                    .iter()
                    .filter(|action| runs_here(&action.platforms))
                    .map(|action| {
                        entry(
                            &action.id,
                            &action.title,
                            menu_contexts(&action.contexts),
                            false,
                        )
                    }),
            );
            entries.extend(
                plugin
                    .panes
                    .iter()
                    .filter(|pane| runs_here(&pane.platforms))
                    .map(|pane| {
                        entry(
                            &pane.id,
                            &pane.title,
                            menu_contexts(&pane.contexts),
                            pane.placement == PluginPanePlacement::Popup,
                        )
                    }),
            );
        }
        entries.retain(|entry| !entry.contexts.is_empty());
        entries.sort_by(|left, right| {
            (&left.title, &left.plugin_id, &left.entry_id).cmp(&(
                &right.title,
                &right.plugin_id,
                &right.entry_id,
            ))
        });
        entries
    }
}

#[cfg(test)]
mod tests {
    use crate::api::schema::InstalledPluginInfo;

    fn plugin(id: &str, enabled: bool, entries: serde_json::Value) -> InstalledPluginInfo {
        let mut plugin = serde_json::json!({
            "plugin_id": id,
            "name": id,
            "version": "0.1.0",
            "manifest_path": "/tmp/herdr-plugin.toml",
            "plugin_root": "/tmp",
            "enabled": enabled,
        });
        plugin
            .as_object_mut()
            .unwrap()
            .extend(entries.as_object().unwrap().clone());
        serde_json::from_value(plugin).unwrap()
    }

    #[test]
    fn lists_menu_entries_of_enabled_plugins_on_this_platform() {
        let mut app = super::super::tests::test_app();
        let foreign = if cfg!(windows) { "linux" } else { "windows" };
        for plugin in [
            plugin(
                "example.priority",
                true,
                serde_json::json!({"actions": [
                    {"id": "toggle", "title": "Toggle priority", "contexts": ["workspace", "global"], "command": ["true"]},
                    {"id": "keys", "title": "Keys only", "contexts": ["global"], "command": ["true"]},
                    {"id": "bare", "title": "No contexts", "command": ["true"]},
                    {"id": "foreign", "title": "Foreign", "contexts": ["workspace"], "platforms": [foreign], "command": ["true"]},
                ]}),
            ),
            plugin(
                "example.groups",
                true,
                serde_json::json!({
                    "actions": [{"id": "assign", "title": "Assign group", "contexts": ["tab"], "command": ["true"]}],
                    "panes": [{"id": "diff", "title": "Diff", "contexts": ["pane", "selection"], "placement": "popup", "command": ["true"]}],
                }),
            ),
            plugin(
                "example.disabled",
                false,
                serde_json::json!({"actions": [
                    {"id": "off", "title": "Disabled", "contexts": ["workspace"], "command": ["true"]},
                ]}),
            ),
        ] {
            app.state
                .installed_plugins
                .insert(plugin.plugin_id.clone(), plugin);
        }

        let entries = app
            .client_shell_plugin_entries()
            .into_iter()
            .map(|entry| {
                format!(
                    "{}.{}: {} {:?} popup={}",
                    entry.plugin_id, entry.entry_id, entry.title, entry.contexts, entry.popup
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            entries,
            [
                "example.groups.assign: Assign group [Tab] popup=false",
                "example.groups.diff: Diff [Pane, Selection] popup=true",
                "example.priority.toggle: Toggle priority [Workspace] popup=false",
            ]
        );
    }
}
