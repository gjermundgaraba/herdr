//! Native space groups: `workspace.set_group` places a space's worktree family
//! in a group, and a moved family takes the group of where it lands. A family
//! is kept together where it shows, so its members never sit apart from it.
//! See `crate::space_order` for the rules.

use std::collections::HashMap;

use crate::api::schema::{
    EventData, EventEnvelope, EventKind, ResponseResult, WorkspaceSetGroupParams,
};
use crate::app::state::AppState;
use crate::app::App;
use crate::space_order::{self, Space, GROUP_TOKEN};

use super::responses::{encode_error, encode_success};

impl AppState {
    /// Every space in server order, as ordering sees it.
    pub(crate) fn order_spaces(&self) -> Vec<Space<'_>> {
        self.workspaces
            .iter()
            .map(|workspace| Space {
                worktree: workspace
                    .worktree_space()
                    .map(|space| (space.key.as_str(), space.is_linked_worktree)),
                group: workspace.metadata_tokens.get(GROUP_TOKEN),
            })
            .collect()
    }

    /// Gathers every worktree family where it shows, for a restored session
    /// whose families may have drifted apart.
    pub(crate) fn gather_worktree_families(&mut self) {
        for index in 0..self.workspaces.len() {
            let (family, before) = self.family_gathering(index, false);
            self.move_family_block(&family, before);
        }
    }

    /// The family `index` belongs to and the space to move it before, `None`
    /// for the end, to gather it at its anchor or where it shows.
    fn family_gathering(&self, index: usize, at_anchor: bool) -> (Vec<usize>, Option<usize>) {
        let spaces = self.order_spaces();
        let family = space_order::family(&spaces, space_order::anchor(&spaces, index));
        let at = if at_anchor {
            family[0]
        } else {
            family.iter().copied().min().unwrap_or(index)
        };
        let before = space_order::gathering(&spaces, &family, at);
        (family, before)
    }

    /// Moves `family` as one block before `before`, or to the end. Returns the
    /// moved and the `before` space ids when the order changed.
    fn move_family_block(
        &mut self,
        family: &[usize],
        before: Option<usize>,
    ) -> Option<(Vec<String>, Option<String>)> {
        let workspace_ids = family
            .iter()
            .map(|member| self.workspaces[*member].id.clone())
            .collect::<Vec<_>>();
        let before_workspace_id = before.map(|before| self.workspaces[before].id.clone());
        self.move_workspace_block(&workspace_ids, before_workspace_id.as_deref())
            .then_some((workspace_ids, before_workspace_id))
    }
}

impl App {
    /// Gathers the worktree family `index` belongs to where it shows, after a
    /// space joined it. Returns where `index` ends up.
    pub(crate) fn gather_worktree_family(&mut self, index: usize) -> usize {
        let id = self.state.workspaces[index].id.clone();
        self.gather_family(index, false);
        self.state
            .workspaces
            .iter()
            .position(|workspace| workspace.id == id)
            .unwrap_or(index)
    }

    /// Gathers the family of each moved space at its anchor, so moving a
    /// checkout brings its worktrees along and a worktree moved alone returns
    /// to its checkout.
    pub(super) fn gather_moved_families(&mut self, moved_ids: &[String]) {
        for moved in moved_ids {
            if let Some(index) = self.state.workspaces.iter().position(|ws| &ws.id == moved) {
                self.gather_family(index, true);
            }
        }
    }

    /// Gathers the family `index` belongs to at its anchor, or where it shows.
    fn gather_family(&mut self, index: usize, at_anchor: bool) {
        let (family, before) = self.state.family_gathering(index, at_anchor);
        self.move_family(&family, before);
    }

    /// Moves `family` as one block before `before`, or to the end.
    fn move_family(&mut self, family: &[usize], before: Option<usize>) {
        if let Some((workspace_ids, before_workspace_id)) =
            self.state.move_family_block(family, before)
        {
            let workspaces = self.workspace_list_info();
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceReordered,
                data: EventData::WorkspaceReordered {
                    workspace_ids,
                    before_workspace_id,
                    workspaces,
                },
            });
        }
    }

    pub(super) fn handle_workspace_set_group(
        &mut self,
        id: String,
        params: WorkspaceSetGroupParams,
    ) -> String {
        let Some(index) = self.parse_workspace_id(&params.workspace_id) else {
            return encode_error(
                id,
                "workspace_not_found",
                format!("workspace {} not found", params.workspace_id),
            );
        };
        let requested = params.group.is_some();
        let group = match super::super::api_helpers::normalize_metadata_tokens(HashMap::from([(
            GROUP_TOKEN.to_owned(),
            params.group,
        )])) {
            Ok(mut tokens) => tokens.remove(GROUP_TOKEN).flatten(),
            Err(message) => return encode_error(id, "invalid_space_group", message),
        };
        if requested && group.is_none() {
            return encode_error(id, "invalid_space_group", "group names cannot be empty");
        }

        let (family, before) = {
            let spaces = self.state.order_spaces();
            let family = space_order::family(&spaces, space_order::anchor(&spaces, index));
            let before = space_order::placement(&spaces, &family, group.as_deref());
            (family, before)
        };
        // Only the anchor carries the family's group.
        let mut changed = false;
        for (position, member) in family.iter().enumerate() {
            let token = if position == 0 { group.clone() } else { None };
            if self.state.workspaces[*member]
                .metadata_tokens
                .patch_persisted(HashMap::from([(GROUP_TOKEN.to_owned(), token)]))
            {
                self.emit_workspace_token_updated(*member);
                changed = true;
            }
        }
        if changed {
            self.schedule_session_save();
        }
        self.move_family(&family, before);
        encode_success(
            id,
            ResponseResult::WorkspaceList {
                workspaces: self.workspace_list_info(),
            },
        )
    }

    /// Gives each moved family the group of where it landed.
    pub(super) fn regroup_moved_spaces(&mut self, moved_ids: &[String]) {
        let changes = {
            let spaces = self.state.order_spaces();
            let mut anchors = Vec::new();
            for moved in moved_ids {
                if let Some(index) = self.state.workspaces.iter().position(|ws| &ws.id == moved) {
                    let anchor = space_order::anchor(&spaces, index);
                    if !anchors.contains(&anchor) {
                        anchors.push(anchor);
                    }
                }
            }
            anchors
                .into_iter()
                .filter_map(|anchor| {
                    let group = space_order::regroup(&spaces, anchor)?;
                    Some((anchor, group.map(str::to_owned)))
                })
                .collect::<Vec<_>>()
        };
        for (anchor, group) in &changes {
            if self.state.workspaces[*anchor]
                .metadata_tokens
                .patch_persisted(HashMap::from([(GROUP_TOKEN.to_owned(), group.clone())]))
            {
                self.emit_workspace_token_updated(*anchor);
            }
        }
        if !changes.is_empty() {
            self.schedule_session_save();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::api::schema::{Method, Request, WorkspaceMoveParams, WorkspaceSetGroupParams};
    use crate::space_order::GROUP_TOKEN;

    fn app(names: &[&str]) -> crate::app::App {
        let mut app = crate::app::App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            tokio::sync::mpsc::unbounded_channel().1,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = names
            .iter()
            .map(|name| crate::workspace::Workspace::test_new(name))
            .collect();
        app.state.ensure_test_terminals();
        app
    }

    fn order(app: &crate::app::App) -> Vec<(String, Option<String>)> {
        app.state
            .workspaces
            .iter()
            .map(|workspace| {
                (
                    workspace.custom_name.clone().unwrap_or_default(),
                    workspace
                        .metadata_tokens
                        .get(GROUP_TOKEN)
                        .map(str::to_owned),
                )
            })
            .collect()
    }

    fn names(app: &crate::app::App) -> Vec<String> {
        order(app).into_iter().map(|(name, _)| name).collect()
    }

    fn move_space(app: &mut crate::app::App, index: usize, insert_index: usize) {
        let workspace_id = app.public_workspace_id(index);
        app.handle_api_request(Request {
            id: "move".into(),
            method: Method::WorkspaceMove(WorkspaceMoveParams {
                workspace_id,
                insert_index,
            }),
        });
    }

    fn set_group(app: &mut crate::app::App, index: usize, group: Option<&str>) {
        let workspace_id = app.public_workspace_id(index);
        let response = app.handle_api_request(Request {
            id: "group".into(),
            method: Method::WorkspaceSetGroup(WorkspaceSetGroupParams {
                workspace_id,
                group: group.map(str::to_owned),
            }),
        });
        assert!(response.contains("workspace_list"), "{response}");
    }

    #[test]
    fn set_group_places_the_space_and_a_drop_inside_a_group_joins_it() {
        let mut app = app(&["one", "two", "three", "four"]);
        let a = || Some("a".to_owned());
        set_group(&mut app, 0, Some("a"));
        set_group(&mut app, 3, Some("a"));
        assert_eq!(
            order(&app),
            [
                ("one".into(), a()),
                ("four".into(), a()),
                ("two".into(), None),
                ("three".into(), None),
            ]
        );
        // The placement set_group made is where regrouping leaves it.
        let four = app.state.workspaces[1].id.clone();
        app.regroup_moved_spaces(&[four]);
        assert_eq!(order(&app)[1], ("four".into(), a()));

        move_space(&mut app, 2, 1);
        assert_eq!(order(&app)[1], ("two".into(), a()));

        set_group(&mut app, 1, None);
        assert_eq!(order(&app)[3], ("two".into(), None));
    }

    #[test]
    fn set_group_cleans_the_name_like_any_metadata_token() {
        let mut app = app(&["one", "two"]);
        set_group(&mut app, 0, Some(" re\u{1b}view\n "));
        assert_eq!(order(&app)[0].1.as_deref(), Some("review"));

        let workspace_id = app.public_workspace_id(1);
        let response = app.handle_api_request(Request {
            id: "group".into(),
            method: Method::WorkspaceSetGroup(WorkspaceSetGroupParams {
                workspace_id,
                group: Some("\u{7}\t".into()),
            }),
        });
        assert!(response.contains("invalid_space_group"), "{response}");
    }

    fn member(checkout: &str, linked: bool) -> crate::workspace::WorktreeSpaceMembership {
        crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "repo".into(),
            repo_root: "/repo".into(),
            checkout_path: checkout.into(),
            is_linked_worktree: linked,
        }
    }

    #[test]
    fn restoring_gathers_drifted_families_where_they_show() {
        let mut app = app(&["one", "checkout", "two", "worktree", "three"]);
        app.state.workspaces[1].worktree_space = Some(member("/repo", false));
        app.state.workspaces[3].worktree_space = Some(member("/repo-wt", true));
        app.state.active = Some(4);

        app.state.gather_worktree_families();

        assert_eq!(names(&app), ["one", "checkout", "worktree", "two", "three"]);
        assert_eq!(app.state.active, Some(4));
    }

    #[test]
    fn a_moved_checkout_brings_its_worktree_and_a_moved_worktree_returns() {
        let mut app = app(&["checkout", "worktree", "x", "y"]);
        app.state.workspaces[0].worktree_space = Some(member("/repo", false));
        app.state.workspaces[1].worktree_space = Some(member("/repo-wt", true));
        move_space(&mut app, 1, 4);
        assert_eq!(names(&app), ["checkout", "worktree", "x", "y"]);
        move_space(&mut app, 0, 4);
        assert_eq!(names(&app), ["x", "y", "checkout", "worktree"]);
    }
}
