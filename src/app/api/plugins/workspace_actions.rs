//! Plugin actions offered on a space's context menu: every action of an
//! enabled plugin that declares the `workspace` context and runs on this
//! platform. The client invokes them with the clicked space as context.

use super::{effective_platforms, platform_supported, plugin_manifest_available};
use crate::api::schema::PluginActionContext;
use crate::app::App;
use crate::protocol::ClientShellPluginAction;

impl App {
    pub(crate) fn client_shell_workspace_actions(&self) -> Vec<ClientShellPluginAction> {
        let mut actions = self
            .state
            .installed_plugins
            .values()
            .filter(|plugin| plugin.enabled && plugin_manifest_available(plugin))
            .flat_map(|plugin| {
                plugin
                    .actions
                    .iter()
                    .filter(|action| {
                        action.contexts.contains(&PluginActionContext::Workspace)
                            && platform_supported(effective_platforms(
                                &action.platforms,
                                &plugin.platforms,
                            ))
                    })
                    .map(|action| ClientShellPluginAction {
                        plugin_id: plugin.plugin_id.clone(),
                        action_id: action.id.clone(),
                        title: action.title.clone(),
                    })
            })
            .collect::<Vec<_>>();
        actions.sort_by(|left, right| {
            (&left.title, &left.plugin_id, &left.action_id).cmp(&(
                &right.title,
                &right.plugin_id,
                &right.action_id,
            ))
        });
        actions
    }
}

#[cfg(test)]
mod tests {
    use crate::api::schema::{InstalledPluginInfo, PluginInvocationContext};
    use crate::app::App;

    fn test_app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        )
    }

    fn plugin(id: &str, enabled: bool, actions: serde_json::Value) -> InstalledPluginInfo {
        serde_json::from_value(serde_json::json!({
            "plugin_id": id,
            "name": id,
            "version": "0.1.0",
            "manifest_path": "/tmp/herdr-plugin.toml",
            "plugin_root": "/tmp",
            "enabled": enabled,
            "actions": actions,
        }))
        .unwrap()
    }

    #[test]
    fn lists_workspace_actions_of_enabled_plugins_on_this_platform() {
        let mut app = test_app();
        let foreign = if cfg!(windows) { "linux" } else { "windows" };
        for plugin in [
            plugin(
                "example.priority",
                true,
                serde_json::json!([
                    {"id": "toggle", "title": "Toggle priority", "contexts": ["workspace"], "command": ["true"]},
                    {"id": "pane", "title": "Pane only", "contexts": ["pane"], "command": ["true"]},
                    {"id": "foreign", "title": "Foreign", "contexts": ["workspace"], "platforms": [foreign], "command": ["true"]},
                ]),
            ),
            plugin(
                "example.groups",
                true,
                serde_json::json!([
                    {"id": "assign", "title": "Assign group", "contexts": ["workspace"], "command": ["true"]},
                ]),
            ),
            plugin(
                "example.disabled",
                false,
                serde_json::json!([
                    {"id": "off", "title": "Disabled", "contexts": ["workspace"], "command": ["true"]},
                ]),
            ),
        ] {
            app.state
                .installed_plugins
                .insert(plugin.plugin_id.clone(), plugin);
        }

        let actions = app
            .client_shell_workspace_actions()
            .into_iter()
            .map(|action| {
                format!(
                    "{}.{}: {}",
                    action.plugin_id, action.action_id, action.title
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actions,
            [
                "example.groups.assign: Assign group",
                "example.priority.toggle: Toggle priority",
            ]
        );
    }

    #[test]
    fn provided_workspace_id_supplies_that_workspace_context() {
        let mut app = test_app();
        app.state.workspaces = vec![
            crate::workspace::Workspace::test_new("focused"),
            crate::workspace::Workspace::test_new("clicked"),
        ];
        app.state.workspaces[1].custom_name = Some("Clicked".into());
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        let clicked = app.public_workspace_id(1);

        let context = app.merge_plugin_context(
            Some(PluginInvocationContext {
                workspace_id: Some(clicked.clone()),
                invocation_source: Some("context_menu".into()),
                ..Default::default()
            }),
            "menu",
        );
        assert_eq!(context.workspace_id, Some(clicked));
        assert_eq!(context.workspace_label.as_deref(), Some("Clicked"));
        assert_eq!(context.tab_id, app.public_tab_id(1, 0));
        assert_eq!(context.invocation_source.as_deref(), Some("context_menu"));
    }
}
