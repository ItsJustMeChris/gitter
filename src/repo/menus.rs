//! Context menus.

use super::diff_view::DiffLocation;
use super::dialogs::ResetTarget;
use super::RepoView;
use crate::assets::Icon;
use crate::git::{short_oid, ChangedFile, FileStatus, LogScope, StatusEntry};
use crate::ui::menu::ContextMenu;
use gpui::{Context, Pixels, Point};

impl RepoView {
    pub fn commit_menu(&self, oid: &str, pos: Point<Pixels>, cx: &mut Context<Self>) -> ContextMenu {
        let short = short_oid(oid).to_string();
        let commit = self.history.visible().iter().find(|c| c.oid == oid).cloned();
        let subject = commit.as_ref().map(|c| c.subject.clone()).unwrap_or_default();
        let is_merge = commit.as_ref().map(|c| c.parents.len() > 1).unwrap_or(false);
        let is_root = commit.as_ref().map(|c| c.parents.is_empty()).unwrap_or(false);
        let on_branch = self.snapshot.head.branch.is_some() && self.repo.is_ancestor(oid, "HEAD");
        let is_head = self.snapshot.head.oid.as_deref() == Some(oid);
        let o = oid.to_string();
        let current = self.snapshot.head.display_name();

        let o1 = o.clone();
        let o2 = o.clone();
        let o3 = o.clone();
        let o4 = o.clone();
        let o5 = o.clone();
        let o6 = o.clone();
        let o7 = o.clone();
        let o8 = o.clone();
        let o9 = o.clone();
        let o10 = o.clone();
        let o11 = o.clone();
        let o12 = o.clone();
        let subject2 = subject.clone();
        let label = format!("{short} · {}", subject.chars().take(40).collect::<String>());

        let mut menu = ContextMenu::new(pos)
            .header(label)
            .item(format!("Check out {short}"), Some(Icon::Commit), self.handler(cx, move |this, window, cx| this.confirm_checkout_detached(o1.clone(), window, cx)))
            .item("Create Branch Here…", Some(Icon::Branch), self.handler(cx, move |this, window, cx| this.open_create_branch(Some(o2.clone()), window, cx)))
            .item("Create Tag Here…", Some(Icon::Tag), self.handler(cx, move |this, window, cx| this.open_create_tag(o3.clone(), window, cx)))
            .separator()
            .item(format!("Cherry-pick onto {current}"), Some(Icon::Cherry), self.handler(cx, move |this, _, cx| {
                let o = o4.clone();
                this.head_op("Cherry-picking", format!("Cherry-picked {}", short_oid(&o)), false, move |repo| repo.cherry_pick(&o), cx);
            }))
            .item("Revert Commit", Some(Icon::Undo), self.handler(cx, move |this, _, cx| {
                let o = o5.clone();
                this.head_op("Reverting", format!("Reverted {}", short_oid(&o)), false, move |repo| repo.revert(&o, is_merge), cx);
            }))
            .item(format!("Reset {current} to Here…"), Some(Icon::Rebase), self.handler(cx, move |this, window, cx| this.open_reset(ResetTarget { oid: o6.clone() }, window, cx)))
            .item(format!("Rebase {current} onto Here"), Some(Icon::Rebase), self.handler(cx, move |this, _, cx| this.rebase_current_onto(o12.clone(), cx)));
        if on_branch && !is_merge {
            menu = menu
                .separator()
                .header("Rewrite history")
                .item("Edit Commit Message…", Some(Icon::Pencil), self.handler(cx, move |this, window, cx| this.open_reword(o7.clone(), window, cx)))
                .item_if(!is_root, "Squash into Parent", Some(Icon::Merge), self.handler(cx, move |this, window, cx| {
                    let o = o8.clone();
                    this.confirm(
                        "Squash into parent?",
                        "This commit will be combined with its parent. Commits on top of it will be rewritten.",
                        "Squash",
                        false,
                        move |this, _, cx| {
                            let o = o.clone();
                            this.head_op("Squashing", "Commits squashed", false, move |repo| repo.squash_into_parent(&o), cx);
                        },
                        window,
                        cx,
                    );
                }))
                .item_if(!is_head, "Move Up", Some(Icon::ArrowUp), self.handler(cx, move |this, _, cx| {
                    let o = o10.clone();
                    this.head_op("Moving commit", "Commit moved", false, move |repo| repo.move_commit(&o, true), cx);
                }))
                .item_if(!is_root, "Move Down", Some(Icon::ArrowDown), self.handler(cx, move |this, _, cx| {
                    let o = o11.clone();
                    this.head_op("Moving commit", "Commit moved", false, move |repo| repo.move_commit(&o, false), cx);
                }))
                .danger_item("Drop Commit…", Some(Icon::Trash), self.handler(cx, move |this, window, cx| {
                    let o = o9.clone();
                    this.confirm(
                        "Drop this commit?",
                        "The commit's changes will be removed from the branch and later commits rewritten.",
                        "Drop Commit",
                        true,
                        move |this, _, cx| {
                            let o = o.clone();
                            this.head_op("Dropping commit", "Commit dropped", false, move |repo| repo.drop_commit(&o), cx);
                        },
                        window,
                        cx,
                    );
                }));
        }
        let o_copy = o.clone();
        menu.separator()
            .item("Copy SHA", Some(Icon::Copy), self.handler(cx, move |this, _, cx| this.copy_to_clipboard(o_copy.clone(), cx)))
            .item("Copy Commit Message", Some(Icon::Copy), self.handler(cx, move |this, _, cx| this.copy_to_clipboard(subject2.clone(), cx)))
    }

    pub fn detail_file_menu(&self, loc: DiffLocation, file: &ChangedFile, pos: Point<Pixels>, cx: &mut Context<Self>) -> ContextMenu {
        let path = file.path.clone();
        let oid = self.detail_state(loc).details.as_ref().map(|d| d.oid.clone());
        let (p1, p2, p3, p4) = (path.clone(), path.clone(), path.clone(), path.clone());
        let f = file.clone();
        let exists = self.repo.workdir.join(&path).exists();
        let mut menu = ContextMenu::new(pos)
            .item("Show File History", Some(Icon::History), self.handler(cx, move |this, window, cx| this.show_history_scope(LogScope::File(p1.clone()), window, cx)))
            .item_if(exists, "Blame", Some(Icon::User), self.handler(cx, move |this, window, cx| this.open_blame(p2.clone(), window, cx)))
            .item_if(exists, "Open File", Some(Icon::Reveal), self.handler(cx, move |this, _, cx| this.open_file(&p3, cx)));
        if let Some(oid) = oid.filter(|_| loc != DiffLocation::Stash && f.status != FileStatus::Deleted) {
            let short = short_oid(&oid).to_string();
            menu = menu.item(format!("Restore File from {short}"), Some(Icon::Undo), self.handler(cx, move |this, _, cx| this.checkout_file_at(oid.clone(), f.clone(), cx)));
        }
        menu.separator().item("Copy Path", Some(Icon::Copy), self.handler(cx, move |this, _, cx| this.copy_to_clipboard(p4.clone(), cx)))
    }

    pub fn wc_file_menu(&self, staged: bool, entry: &StatusEntry, pos: Point<Pixels>, cx: &mut Context<Self>) -> ContextMenu {
        let path = entry.path.clone();
        let e = entry.clone();
        let (p1, p2, p3, p4, p5, p6, p7) = (path.clone(), path.clone(), path.clone(), path.clone(), path.clone(), path.clone(), path.clone());
        let mut menu = ContextMenu::new(pos);
        if entry.status == FileStatus::Conflicted {
            let (c1, c2, c3) = (path.clone(), path.clone(), path.clone());
            menu = menu
                .header("Conflict")
                .item("Resolve Using My Version", Some(Icon::Check), self.handler(cx, move |this, _, cx| this.resolve_conflict(c1.clone(), true, cx)))
                .item("Resolve Using Their Version", Some(Icon::Check), self.handler(cx, move |this, _, cx| this.resolve_conflict(c2.clone(), false, cx)))
                .item("Mark as Resolved", Some(Icon::Check), self.handler(cx, move |this, _, cx| this.mark_resolved(c3.clone(), cx)))
                .separator();
        } else if staged {
            menu = menu.item("Unstage", Some(Icon::Minus), self.handler(cx, move |this, _, cx| this.unstage_paths(vec![p1.clone()], cx)));
        } else {
            menu = menu.item("Stage", Some(Icon::Plus), self.handler(cx, move |this, _, cx| this.stage_paths(vec![p1.clone()], cx))).danger_item(
                "Discard Changes…",
                Some(Icon::Undo),
                self.handler(cx, move |this, window, cx| this.confirm_discard(vec![e.clone()], window, cx)),
            );
        }
        let ext = std::path::Path::new(&path).extension().map(|x| format!("*.{}", x.to_string_lossy()));
        let dir = path.rfind('/').map(|i| format!("{}/", &path[..i]));
        menu = menu
            .separator()
            .item("Open File", Some(Icon::Reveal), self.handler(cx, move |this, _, cx| this.open_file(&p2, cx)))
            .item("Show in File Manager", Some(Icon::Folder), self.handler(cx, move |this, _, cx| this.reveal_file(&p3, cx)))
            .item_if(entry.status != FileStatus::Untracked, "Show File History", Some(Icon::History), self.handler(cx, move |this, window, cx| this.show_history_scope(LogScope::File(p4.clone()), window, cx)))
            .item_if(entry.status != FileStatus::Untracked && entry.status != FileStatus::Added, "Blame", Some(Icon::User), self.handler(cx, move |this, window, cx| this.open_blame(p5.clone(), window, cx)));
        if entry.status == FileStatus::Untracked {
            menu = menu.separator().header("Ignore").item(format!("Ignore “{path}”"), Some(Icon::Eye), self.handler(cx, move |this, _, cx| this.ignore_path(p6.clone(), cx)));
            if let Some(ext) = ext {
                let ext2 = ext.clone();
                menu = menu.item(format!("Ignore All {ext} Files"), Some(Icon::Eye), self.handler(cx, move |this, _, cx| this.ignore_path(ext2.clone(), cx)));
            }
            if let Some(dir) = dir {
                let d2 = dir.clone();
                menu = menu.item(format!("Ignore Folder “{dir}”"), Some(Icon::Eye), self.handler(cx, move |this, _, cx| this.ignore_path(d2.clone(), cx)));
            }
        }
        menu.separator().item("Copy Path", Some(Icon::Copy), self.handler(cx, move |this, _, cx| this.copy_to_clipboard(p7.clone(), cx)))
    }

    pub fn branch_menu(&self, name: &str, pos: Point<Pixels>, cx: &mut Context<Self>) -> ContextMenu {
        let branch = self.snapshot.branches.iter().find(|b| b.name == name).cloned();
        let is_head = branch.as_ref().map(|b| b.is_head).unwrap_or(false);
        let upstream = branch.as_ref().and_then(|b| b.upstream.clone());
        let current = self.snapshot.head.display_name();
        let n = name.to_string();
        let mut menu = ContextMenu::new(pos).header(name.to_string());
        if !is_head {
            let (n1, n2, n3) = (n.clone(), n.clone(), n.clone());
            menu = menu
                .item(format!("Check Out “{name}”"), Some(Icon::Check), self.handler(cx, move |this, _, cx| this.checkout_branch(n1.clone(), cx)))
                .item(format!("Merge “{name}” into “{current}”"), Some(Icon::Merge), self.handler(cx, move |this, window, cx| this.open_merge_dialog(n2.clone(), window, cx)))
                .item(format!("Rebase “{current}” onto “{name}”"), Some(Icon::Rebase), self.handler(cx, move |this, _, cx| this.rebase_current_onto(n3.clone(), cx)))
                .separator();
        }
        let (n4, n5, n6, n7) = (n.clone(), n.clone(), n.clone(), n.clone());
        menu = menu
            .item("Push…", Some(Icon::Push), self.handler(cx, move |this, window, cx| this.open_push_dialog(n4.clone(), window, cx)))
            .item_if(is_head && upstream.is_some(), "Pull", Some(Icon::Pull), self.handler(cx, |this, _, cx| this.pull(cx)))
            .separator()
            .item("Create Branch from Here…", Some(Icon::Branch), self.handler(cx, move |this, window, cx| this.open_create_branch(Some(n5.clone()), window, cx)))
            .item("Show History", Some(Icon::History), self.handler(cx, move |this, window, cx| this.show_history_scope(LogScope::Ref(n6.clone()), window, cx)))
            .item("Rename…", Some(Icon::Pencil), self.handler(cx, move |this, window, cx| this.open_rename_branch(n7.clone(), window, cx)));
        if let Some(up) = upstream {
            let n = n.clone();
            menu = menu.item(format!("Stop Tracking {up}"), Some(Icon::Close), self.handler(cx, move |this, _, cx| {
                let n = n.clone();
                this.simple_op("Updating upstream", "Upstream removed", move |repo| repo.set_upstream(&n, None), cx);
            }));
        } else if let Some(remote) = self.snapshot.remotes.first() {
            let candidate = format!("{}/{}", remote.name, name);
            if self.snapshot.remote_branches.iter().any(|r| r.full_name == candidate) {
                let n = n.clone();
                menu = menu.item(format!("Track {candidate}"), Some(Icon::Cloud), self.handler(cx, move |this, _, cx| {
                    let (n, c) = (n.clone(), candidate.clone());
                    this.simple_op("Updating upstream", format!("Now tracking {c}"), move |repo| repo.set_upstream(&n, Some(&c)), cx);
                }));
            }
        }
        let n8 = n.clone();
        menu = menu.separator().item("Copy Branch Name", Some(Icon::Copy), self.handler(cx, move |this, _, cx| this.copy_to_clipboard(n8.clone(), cx)));
        if !is_head {
            menu = menu.danger_item(format!("Delete “{name}”…"), Some(Icon::Trash), self.handler(cx, move |this, window, cx| this.confirm_delete_branch(n.clone(), window, cx)));
        }
        menu
    }

    pub fn remote_branch_menu(&self, full: &str, pos: Point<Pixels>, cx: &mut Context<Self>) -> ContextMenu {
        let (remote, branch) = full.split_once('/').map(|(r, b)| (r.to_string(), b.to_string())).unwrap_or((String::new(), full.to_string()));
        let current = self.snapshot.head.display_name();
        let f = full.to_string();
        let (f1, f2, f3, f4, f5, f6) = (f.clone(), f.clone(), f.clone(), f.clone(), f.clone(), f.clone());
        let local_exists = self.snapshot.branches.iter().any(|b| b.name == branch);
        ContextMenu::new(pos)
            .header(full.to_string())
            .item(if local_exists { format!("Check Out “{branch}”") } else { format!("Check Out as New Tracking Branch “{branch}”") }, Some(Icon::Check), self.handler(cx, move |this, _, cx| this.checkout_remote_branch(f1.clone(), cx)))
            .item(format!("Merge into “{current}”"), Some(Icon::Merge), self.handler(cx, move |this, window, cx| this.open_merge_dialog(f2.clone(), window, cx)))
            .item(format!("Rebase “{current}” onto “{full}”"), Some(Icon::Rebase), self.handler(cx, move |this, _, cx| this.rebase_current_onto(f3.clone(), cx)))
            .separator()
            .item("Create Branch from Here…", Some(Icon::Branch), self.handler(cx, move |this, window, cx| this.open_create_branch(Some(f4.clone()), window, cx)))
            .item("Show History", Some(Icon::History), self.handler(cx, move |this, window, cx| this.show_history_scope(LogScope::Ref(f5.clone()), window, cx)))
            .item("Copy Name", Some(Icon::Copy), self.handler(cx, move |this, _, cx| this.copy_to_clipboard(f6.clone(), cx)))
            .separator()
            .danger_item(format!("Delete “{branch}” on {remote}…"), Some(Icon::Trash), self.handler(cx, move |this, window, cx| {
                let (r, b) = (remote.clone(), branch.clone());
                this.confirm(
                    "Delete remote branch?",
                    format!("“{b}” will be deleted from the remote “{r}”. Anyone else using this branch will be affected."),
                    "Delete",
                    true,
                    move |this, _, cx| {
                        let (r, b) = (r.clone(), b.clone());
                        this.run_progress_op(format!("Deleting {r}/{b}"), format!("Deleted {r}/{b}"), move |repo, p| repo.delete_remote_branch(&r, &b, p), cx);
                    },
                    window,
                    cx,
                );
            }))
    }

    pub fn remote_menu(&self, name: &str, pos: Point<Pixels>, cx: &mut Context<Self>) -> ContextMenu {
        let n = name.to_string();
        let (n1, n2, n3) = (n.clone(), n.clone(), n.clone());
        let url = self.snapshot.remotes.iter().find(|r| r.name == name).map(|r| r.fetch_url.clone()).unwrap_or_default();
        let url2 = url.clone();
        let web = web_url(&url);
        let mut menu = ContextMenu::new(pos).header(name.to_string()).item(format!("Fetch “{name}”"), Some(Icon::Fetch), self.handler(cx, move |this, _, cx| this.fetch(Some(n1.clone()), cx)));
        if let Some(web) = web {
            menu = menu.item("Open in Browser", Some(Icon::Globe), self.handler(cx, move |_, _, cx| cx.open_url(&web)));
        }
        menu.item("Edit Remote…", Some(Icon::Pencil), self.handler(cx, move |this, window, cx| this.open_edit_remote(n2.clone(), url2.clone(), window, cx)))
            .item("Copy URL", Some(Icon::Copy), self.handler(cx, move |this, _, cx| this.copy_to_clipboard(url.clone(), cx)))
            .separator()
            .danger_item("Remove Remote…", Some(Icon::Trash), self.handler(cx, move |this, window, cx| {
                let n = n3.clone();
                this.confirm(
                    "Remove remote?",
                    format!("The remote “{n}” and its remote-tracking branches will be removed from this repository. Nothing on the server is deleted."),
                    "Remove",
                    true,
                    move |this, _, cx| {
                        let n = n.clone();
                        this.simple_op("Removing remote", "Remote removed", move |repo| repo.remove_remote(&n), cx);
                    },
                    window,
                    cx,
                );
            }))
    }

    pub fn tag_menu(&self, name: &str, pos: Point<Pixels>, cx: &mut Context<Self>) -> ContextMenu {
        let n = name.to_string();
        let tag = self.snapshot.tags.iter().find(|t| t.name == name).cloned();
        let (n1, n2, n3, n4, n5) = (n.clone(), n.clone(), n.clone(), n.clone(), n.clone());
        let remote = self.snapshot.remotes.first().map(|r| r.name.clone());
        let oid = tag.map(|t| t.oid);
        let mut menu = ContextMenu::new(pos).header(name.to_string());
        if let Some(oid) = oid.clone() {
            menu = menu
                .item("Show Commit", Some(Icon::Commit), self.handler(cx, move |this, window, cx| this.reveal_commit(&oid, window, cx)))
                .item(format!("Check Out “{name}”"), Some(Icon::Check), self.handler(cx, move |this, window, cx| this.confirm_checkout_detached(n1.clone(), window, cx)));
        }
        menu = menu.item("Create Branch from Tag…", Some(Icon::Branch), self.handler(cx, move |this, window, cx| this.open_create_branch(Some(format!("refs/tags/{}", n2)), window, cx)));
        if let Some(remote) = remote {
            let r1 = remote.clone();
            let r2 = remote.clone();
            menu = menu.separator().item(format!("Push to {remote}"), Some(Icon::Push), self.handler(cx, move |this, _, cx| {
                let (r, t) = (r1.clone(), n3.clone());
                this.run_progress_op(format!("Pushing tag {t}"), format!("Pushed tag {t} to {r}"), move |repo, p| repo.push_tag(&r, &t, p), cx);
            }))
            .danger_item(format!("Delete from {remote}…"), Some(Icon::Trash), self.handler(cx, move |this, window, cx| {
                let (r, t) = (r2.clone(), n4.clone());
                this.confirm("Delete remote tag?", format!("The tag “{t}” will be deleted from “{r}”."), "Delete", true, move |this, _, cx| {
                    let (r, t) = (r.clone(), t.clone());
                    this.run_progress_op(format!("Deleting tag {t}"), format!("Deleted tag {t} from {r}"), move |repo, p| repo.delete_remote_tag(&r, &t, p), cx);
                }, window, cx);
            }));
        }
        menu.separator().danger_item("Delete Tag…", Some(Icon::Trash), self.handler(cx, move |this, window, cx| {
            let t = n5.clone();
            this.confirm("Delete tag?", format!("The local tag “{t}” will be deleted."), "Delete", true, move |this, _, cx| {
                let t = t.clone();
                let oid = this.snapshot.tags.iter().find(|x| x.name == t).map(|x| x.oid.clone());
                let t2 = t.clone();
                this.run_op("Deleting tag", move |repo| repo.delete_tag(&t2), move |this, r, cx| match r {
                    Ok(_) => {
                        if let Some(oid) = oid.clone() {
                            this.push_undo(format!("Delete tag {t}"), super::undo::UndoAction::DeleteTag { name: t.clone(), oid });
                        }
                        this.toast_undoable(format!("Deleted tag {t}"), cx);
                    }
                    Err(e) => this.show_error("Delete tag", &e, cx),
                }, cx);
            }, window, cx);
        }))
    }

    pub fn submodule_menu(&self, path: &str, pos: Point<Pixels>, cx: &mut Context<Self>) -> ContextMenu {
        let p = path.to_string();
        let p2 = p.clone();
        ContextMenu::new(pos)
            .item("Open Submodule", Some(Icon::Repo), self.handler(cx, move |this, _, cx| {
                let full = this.repo.workdir.join(&p);
                cx.emit(super::RepoEvent::OpenRepository(full));
            }))
            .item("Show in File Manager", Some(Icon::Folder), self.handler(cx, move |this, _, cx| this.reveal_file(&p2, cx)))
            .item("Update All Submodules", Some(Icon::Refresh), self.handler(cx, |this, _, cx| {
                this.run_progress_op("Updating submodules", "Submodules updated", |repo, p| repo.submodule_update(p), cx);
            }))
    }

    pub fn stash_menu(&self, index: usize, pos: Point<Pixels>, cx: &mut Context<Self>) -> ContextMenu {
        let Some(stash) = self.snapshot.stashes.get(index).cloned() else { return ContextMenu::new(pos) };
        let (s1, s2, s3) = (stash.clone(), stash.clone(), stash.clone());
        ContextMenu::new(pos)
            .header(stash.message.clone())
            .item("Apply Stash", Some(Icon::StashPop), self.handler(cx, move |this, _, cx| this.apply_stash(s1.clone(), false, cx)))
            .item("Apply and Remove (Pop)", Some(Icon::StashPop), self.handler(cx, move |this, _, cx| this.apply_stash(s2.clone(), true, cx)))
            .separator()
            .danger_item("Delete Stash…", Some(Icon::Trash), self.handler(cx, move |this, window, cx| this.confirm_drop_stash(s3.clone(), window, cx)))
    }

    pub fn reflog_menu(&self, ix: usize, pos: Point<Pixels>, cx: &mut Context<Self>) -> ContextMenu {
        let Some(entry) = self.reflog.entries.get(ix).cloned() else { return ContextMenu::new(pos) };
        let o = entry.oid.clone();
        let (o1, o2, o3, o4, o5) = (o.clone(), o.clone(), o.clone(), o.clone(), o.clone());
        ContextMenu::new(pos)
            .header(format!("{} · {}", entry.selector, short_oid(&entry.oid)))
            .item("Show in History", Some(Icon::History), self.handler(cx, move |this, window, cx| this.reveal_commit(&o1, window, cx)))
            .item("Check Out", Some(Icon::Commit), self.handler(cx, move |this, window, cx| this.confirm_checkout_detached(o2.clone(), window, cx)))
            .item("Create Branch Here…", Some(Icon::Branch), self.handler(cx, move |this, window, cx| this.open_create_branch(Some(o3.clone()), window, cx)))
            .item("Reset Current Branch to Here…", Some(Icon::Rebase), self.handler(cx, move |this, window, cx| this.open_reset(ResetTarget { oid: o4.clone() }, window, cx)))
            .separator()
            .item("Copy SHA", Some(Icon::Copy), self.handler(cx, move |this, _, cx| this.copy_to_clipboard(o5.clone(), cx)))
    }

}

/// Converts a git remote URL into a browsable https URL, when possible.
pub fn web_url(url: &str) -> Option<String> {
    let url = url.trim();
    let converted = if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        format!("https://{host}/{path}")
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
        let (host, path) = rest.split_once('/')?;
        let host = host.split(':').next().unwrap_or(host);
        format!("https://{host}/{path}")
    } else if url.starts_with("https://") || url.starts_with("http://") {
        // Strip embedded credentials.
        match url.split_once("://").and_then(|(scheme, rest)| rest.split_once('@').map(|(_, r)| format!("{scheme}://{r}"))) {
            Some(u) => u,
            None => url.to_string(),
        }
    } else {
        return None;
    };
    Some(converted.trim_end_matches(".git").to_string())
}
