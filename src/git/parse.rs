//! Data types and parsers for git's machine-readable output formats.

use super::Oid;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeadInfo {
    /// Current branch name, `None` when detached.
    pub branch: Option<String>,
    /// Commit HEAD points to, `None` on an unborn branch.
    pub oid: Option<Oid>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub detached: bool,
}

impl HeadInfo {
    pub fn display_name(&self) -> String {
        match (&self.branch, &self.oid) {
            (Some(b), _) => b.clone(),
            (None, Some(oid)) => format!("Detached at {}", short_oid(oid)),
            (None, None) => "No branch".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
    Unknown,
}

impl FileStatus {
    pub fn from_code(c: char) -> FileStatus {
        match c {
            'A' => FileStatus::Added,
            'M' => FileStatus::Modified,
            'D' => FileStatus::Deleted,
            'R' => FileStatus::Renamed,
            'C' => FileStatus::Copied,
            'T' => FileStatus::TypeChanged,
            'U' => FileStatus::Conflicted,
            '?' => FileStatus::Untracked,
            _ => FileStatus::Unknown,
        }
    }

    pub fn letter(self) -> &'static str {
        match self {
            FileStatus::Added => "A",
            FileStatus::Modified => "M",
            FileStatus::Deleted => "D",
            FileStatus::Renamed => "R",
            FileStatus::Copied => "C",
            FileStatus::TypeChanged => "T",
            FileStatus::Untracked => "?",
            FileStatus::Conflicted => "!",
            FileStatus::Unknown => "·",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FileStatus::Added => "Added",
            FileStatus::Modified => "Modified",
            FileStatus::Deleted => "Deleted",
            FileStatus::Renamed => "Renamed",
            FileStatus::Copied => "Copied",
            FileStatus::TypeChanged => "Type changed",
            FileStatus::Untracked => "Untracked",
            FileStatus::Conflicted => "Conflicted",
            FileStatus::Unknown => "Changed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    BothModified,
    BothAdded,
    BothDeleted,
    AddedByUs,
    AddedByThem,
    DeletedByUs,
    DeletedByThem,
}

impl ConflictKind {
    fn from_xy(xy: &str) -> ConflictKind {
        match xy {
            "DD" => ConflictKind::BothDeleted,
            "AU" => ConflictKind::AddedByUs,
            "UD" => ConflictKind::DeletedByThem,
            "UA" => ConflictKind::AddedByThem,
            "DU" => ConflictKind::DeletedByUs,
            "AA" => ConflictKind::BothAdded,
            _ => ConflictKind::BothModified,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ConflictKind::BothModified => "Both modified",
            ConflictKind::BothAdded => "Both added",
            ConflictKind::BothDeleted => "Both deleted",
            ConflictKind::AddedByUs => "Added by us",
            ConflictKind::AddedByThem => "Added by them",
            ConflictKind::DeletedByUs => "Deleted by us",
            ConflictKind::DeletedByThem => "Deleted by them",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatusEntry {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub conflict: Option<ConflictKind>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkingStatus {
    pub staged: Vec<StatusEntry>,
    pub unstaged: Vec<StatusEntry>,
}

impl WorkingStatus {
    pub fn conflicted(&self) -> impl Iterator<Item = &StatusEntry> {
        self.unstaged.iter().filter(|e| e.status == FileStatus::Conflicted)
    }

    pub fn has_conflicts(&self) -> bool {
        self.conflicted().next().is_some()
    }

    pub fn change_count(&self) -> usize {
        let mut paths: Vec<&str> = self.staged.iter().chain(self.unstaged.iter()).map(|e| e.path.as_str()).collect();
        paths.sort_unstable();
        paths.dedup();
        paths.len()
    }

    pub fn is_clean(&self) -> bool {
        self.staged.is_empty() && self.unstaged.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Branch {
    pub name: String,
    pub oid: Oid,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub upstream_gone: bool,
    pub is_head: bool,
    pub subject: String,
    pub time: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteBranch {
    pub remote: String,
    /// Branch name without the remote prefix.
    pub name: String,
    /// e.g. "origin/main"
    pub full_name: String,
    pub oid: Oid,
    pub time: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub name: String,
    /// The commit the tag points to (peeled).
    pub oid: Oid,
    pub annotated: bool,
    pub subject: String,
    pub time: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Remote {
    pub name: String,
    pub fetch_url: String,
    pub push_url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stash {
    pub index: usize,
    /// e.g. "stash@{0}"
    pub name: String,
    pub oid: Oid,
    pub message: String,
    pub time: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Submodule {
    pub path: String,
    pub oid: Oid,
    /// ' ' up to date, '-' not initialized, '+' checked out commit differs, 'U' conflicts.
    pub state: char,
    pub describe: String,
}

#[derive(Clone, Debug, Default)]
pub struct Refs {
    pub branches: Vec<Branch>,
    pub remote_branches: Vec<RemoteBranch>,
    pub tags: Vec<Tag>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefKind {
    Head = 0,
    Local = 1,
    Remote = 2,
    Tag = 3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
    pub is_head: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    pub oid: Oid,
    pub parents: Vec<Oid>,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub subject: String,
}

impl Commit {
    pub fn short(&self) -> &str {
        short_oid(&self.oid)
    }
}

#[derive(Clone, Debug)]
pub struct CommitDetails {
    pub oid: Oid,
    pub parents: Vec<Oid>,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub committer_name: String,
    pub committer_email: String,
    pub committer_time: i64,
    pub message: String,
    pub files: Vec<ChangedFile>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChangedFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct ReflogEntry {
    pub oid: Oid,
    pub selector: String,
    pub subject: String,
    pub time: i64,
}

#[derive(Clone, Debug)]
pub struct BlameLine {
    pub oid: Oid,
    pub author: String,
    pub time: i64,
    pub summary: String,
    pub line_no: u32,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// Line content without the leading +/-/space, and without the trailing newline.
    /// A trailing '\r' is preserved so patches round-trip exactly.
    pub content: String,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    /// The line is followed by "\ No newline at end of file".
    pub no_newline: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hunk {
    pub header: String,
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    /// The function context git printed after the @@ markers.
    pub fn context(&self) -> &str {
        self.header.splitn(3, "@@").nth(2).map(|s| s.trim()).unwrap_or("")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    /// Raw header lines ("diff --git", "index", "---", "+++", mode lines).
    pub header: Vec<String>,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
    pub new_file: bool,
    pub deleted: bool,
    /// The file has unresolved conflict markers; removed lines are "ours" and added lines "theirs".
    pub conflict: bool,
    /// Old and new versions of an image file, for a visual diff.
    pub images: Option<super::images::ImageVersions>,
}

impl FileDiff {
    pub fn empty(path: &str) -> FileDiff {
        FileDiff { path: path.to_string(), ..Default::default() }
    }

    pub fn stats(&self) -> (usize, usize) {
        let mut add = 0;
        let mut del = 0;
        for h in &self.hunks {
            for l in &h.lines {
                match l.kind {
                    LineKind::Added => add += 1,
                    LineKind::Removed => del += 1,
                    LineKind::Context => {}
                }
            }
        }
        (add, del)
    }

}

pub fn short_oid(oid: &str) -> &str {
    &oid[..oid.len().min(8)]
}

// -------------------------------------------------------------------------
// status --porcelain=v2 -z --branch
// -------------------------------------------------------------------------

pub fn parse_status(bytes: &[u8]) -> (HeadInfo, WorkingStatus) {
    let mut head = HeadInfo::default();
    let mut status = WorkingStatus::default();
    let text = String::from_utf8_lossy(bytes);
    let mut records = text.split('\0').filter(|r| !r.is_empty());
    while let Some(rec) = records.next() {
        if let Some(header) = rec.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" => head.oid = (value != "(initial)").then(|| value.to_string()),
                "branch.head" => {
                    if value == "(detached)" {
                        head.detached = true;
                    } else {
                        head.branch = Some(value.to_string());
                    }
                }
                "branch.upstream" => head.upstream = Some(value.to_string()),
                "branch.ab" => {
                    for part in value.split_whitespace() {
                        if let Some(n) = part.strip_prefix('+') {
                            head.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            head.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let kind = rec.as_bytes()[0];
        match kind {
            b'1' => {
                let parts: Vec<&str> = rec.splitn(9, ' ').collect();
                if parts.len() < 9 {
                    continue;
                }
                push_changes(&mut status, parts[1], parts[8], None);
            }
            b'2' => {
                let parts: Vec<&str> = rec.splitn(10, ' ').collect();
                if parts.len() < 10 {
                    continue;
                }
                let orig = records.next().map(|s| s.to_string());
                push_changes(&mut status, parts[1], parts[9], orig);
            }
            b'u' => {
                let parts: Vec<&str> = rec.splitn(11, ' ').collect();
                if parts.len() < 11 {
                    continue;
                }
                status.unstaged.push(StatusEntry {
                    path: parts[10].to_string(),
                    old_path: None,
                    status: FileStatus::Conflicted,
                    conflict: Some(ConflictKind::from_xy(parts[1])),
                });
            }
            b'?' => {
                status.unstaged.push(StatusEntry { path: rec[2..].to_string(), old_path: None, status: FileStatus::Untracked, conflict: None });
            }
            _ => {}
        }
    }
    let by_path = |a: &StatusEntry, b: &StatusEntry| a.path.to_lowercase().cmp(&b.path.to_lowercase());
    status.staged.sort_by(by_path);
    status.unstaged.sort_by(|a, b| {
        // Conflicts first, then tracked changes, then untracked files.
        let rank = |e: &StatusEntry| match e.status {
            FileStatus::Conflicted => 0,
            FileStatus::Untracked => 2,
            _ => 1,
        };
        rank(a).cmp(&rank(b)).then_with(|| by_path(a, b))
    });
    (head, status)
}

fn push_changes(status: &mut WorkingStatus, xy: &str, path: &str, orig: Option<String>) {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');
    if x != '.' {
        let st = FileStatus::from_code(x);
        status.staged.push(StatusEntry {
            path: path.to_string(),
            old_path: if matches!(st, FileStatus::Renamed | FileStatus::Copied) { orig.clone() } else { None },
            status: st,
            conflict: None,
        });
    }
    if y != '.' {
        status.unstaged.push(StatusEntry { path: path.to_string(), old_path: None, status: FileStatus::from_code(y), conflict: None });
    }
}

// -------------------------------------------------------------------------
// for-each-ref
// -------------------------------------------------------------------------

pub fn parse_refs(out: &str) -> Refs {
    let mut refs = Refs::default();
    for line in out.lines() {
        let f: Vec<&str> = line.split('\0').collect();
        if f.len() < 9 {
            continue;
        }
        let (refname, oid, peeled, upstream, track, head, date, subject, symref) = (f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7], f[8]);
        let time = date.parse().unwrap_or(0);
        if let Some(name) = refname.strip_prefix("refs/heads/") {
            let (ahead, behind, gone) = parse_track(track);
            refs.branches.push(Branch {
                name: name.to_string(),
                oid: oid.to_string(),
                upstream: (!upstream.is_empty()).then(|| upstream.to_string()),
                ahead,
                behind,
                upstream_gone: gone,
                is_head: head == "*",
                subject: subject.to_string(),
                time,
            });
        } else if let Some(full) = refname.strip_prefix("refs/remotes/") {
            if !symref.is_empty() || full.ends_with("/HEAD") {
                continue;
            }
            let (remote, name) = full.split_once('/').unwrap_or((full, ""));
            refs.remote_branches.push(RemoteBranch { remote: remote.to_string(), name: name.to_string(), full_name: full.to_string(), oid: oid.to_string(), time });
        } else if let Some(name) = refname.strip_prefix("refs/tags/") {
            let annotated = !peeled.is_empty();
            refs.tags.push(Tag { name: name.to_string(), oid: if annotated { peeled.to_string() } else { oid.to_string() }, annotated, subject: subject.to_string(), time });
        }
    }
    // Newest tags first, like most clients.
    refs.tags.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| b.name.cmp(&a.name)));
    refs
}

fn parse_track(track: &str) -> (u32, u32, bool) {
    if track.contains("gone") {
        return (0, 0, true);
    }
    let mut ahead = 0;
    let mut behind = 0;
    for part in track.split(',') {
        let part = part.trim();
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().unwrap_or(0);
        }
    }
    (ahead, behind, false)
}

// -------------------------------------------------------------------------
// remote -v
// -------------------------------------------------------------------------

pub fn parse_remotes(out: &str) -> Vec<Remote> {
    let mut remotes: Vec<Remote> = Vec::new();
    for line in out.lines() {
        let mut parts = line.split_whitespace();
        let (Some(name), Some(url)) = (parts.next(), parts.next()) else { continue };
        let kind = parts.next().unwrap_or("(fetch)");
        let remote = match remotes.iter_mut().find(|r| r.name == name) {
            Some(r) => r,
            None => {
                remotes.push(Remote { name: name.to_string(), fetch_url: String::new(), push_url: String::new() });
                remotes.last_mut().unwrap()
            }
        };
        if kind.contains("push") {
            remote.push_url = url.to_string();
        } else {
            remote.fetch_url = url.to_string();
        }
    }
    remotes
}

// -------------------------------------------------------------------------
// log / stash list / reflog
// -------------------------------------------------------------------------

pub fn parse_log(out: &str) -> Vec<Commit> {
    out.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(6, '\x1f').collect();
            if f.len() < 6 || f[0].len() < 7 {
                return None;
            }
            Some(Commit {
                oid: f[0].to_string(),
                parents: f[1].split_whitespace().map(String::from).collect(),
                author_name: f[2].to_string(),
                author_email: f[3].to_string(),
                author_time: f[4].parse().unwrap_or(0),
                subject: f[5].to_string(),
            })
        })
        .collect()
}

pub fn parse_stashes(out: &str) -> Vec<Stash> {
    out.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(4, '\x1f').collect();
            if f.len() < 4 {
                return None;
            }
            let index = f[0].trim_start_matches("stash@{").trim_end_matches('}').parse().unwrap_or(0);
            Some(Stash { index, name: f[0].to_string(), oid: f[1].to_string(), time: f[2].parse().unwrap_or(0), message: f[3].to_string() })
        })
        .collect()
}

pub fn parse_reflog(out: &str) -> Vec<ReflogEntry> {
    let mut entries: Vec<ReflogEntry> = out
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(3, '\x1f').collect();
            if f.len() < 3 {
                return None;
            }
            // "HEAD@{1700000000}" -> ref name and entry time.
            let (name, rest) = f[1].split_once("@{").unwrap_or((f[1], ""));
            let time = rest.trim_end_matches('}').parse().unwrap_or(0);
            Some(ReflogEntry { oid: f[0].to_string(), selector: name.to_string(), subject: f[2].to_string(), time })
        })
        .collect();
    for (i, e) in entries.iter_mut().enumerate() {
        e.selector = format!("{}@{{{i}}}", e.selector);
    }
    entries
}

pub fn parse_submodules(out: &str) -> Vec<Submodule> {
    out.lines()
        .filter_map(|line| {
            if line.len() < 42 {
                return None;
            }
            let state = line.chars().next()?;
            let rest = &line[1..];
            let mut parts = rest.splitn(2, ' ');
            let oid = parts.next()?.to_string();
            let rest = parts.next().unwrap_or("");
            let (path, describe) = match rest.rfind(" (") {
                Some(i) if rest.ends_with(')') => (&rest[..i], &rest[i + 2..rest.len() - 1]),
                _ => (rest, ""),
            };
            Some(Submodule { path: path.to_string(), oid, state, describe: describe.to_string() })
        })
        .collect()
}

pub fn parse_blame(out: &str) -> Vec<BlameLine> {
    use std::collections::HashMap;
    let mut lines = Vec::new();
    let mut info: HashMap<String, (String, i64, String)> = HashMap::new();
    let mut current_oid = String::new();
    let mut current_line = 0u32;
    let mut author = String::new();
    let mut time = 0i64;
    let mut summary = String::new();
    for line in out.lines() {
        if let Some(content) = line.strip_prefix('\t') {
            let entry = info.entry(current_oid.clone()).or_insert_with(|| (author.clone(), time, summary.clone()));
            lines.push(BlameLine { oid: current_oid.clone(), author: entry.0.clone(), time: entry.1, summary: entry.2.clone(), line_no: current_line, content: content.to_string() });
            continue;
        }
        let mut parts = line.split(' ');
        let first = parts.next().unwrap_or("");
        if first.len() >= 40 && first.chars().all(|c| c.is_ascii_hexdigit()) {
            current_oid = first.to_string();
            let _orig = parts.next();
            current_line = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            if let Some(known) = info.get(&current_oid) {
                author = known.0.clone();
                time = known.1;
                summary = known.2.clone();
            }
        } else if let Some(a) = line.strip_prefix("author ") {
            author = a.to_string();
        } else if let Some(t) = line.strip_prefix("author-time ") {
            time = t.parse().unwrap_or(0);
        } else if let Some(s) = line.strip_prefix("summary ") {
            summary = s.to_string();
        }
    }
    lines
}

// -------------------------------------------------------------------------
// diff-tree --name-status -z / --numstat -z
// -------------------------------------------------------------------------

pub fn parse_name_status(bytes: &[u8]) -> Vec<ChangedFile> {
    let text = String::from_utf8_lossy(bytes);
    let mut fields = text.split('\0').filter(|s| !s.is_empty());
    let mut files = Vec::new();
    while let Some(code) = fields.next() {
        let c = code.chars().next().unwrap_or('M');
        let status = FileStatus::from_code(c);
        if matches!(status, FileStatus::Renamed | FileStatus::Copied) {
            let old = fields.next().unwrap_or("").to_string();
            let new = fields.next().unwrap_or("").to_string();
            files.push(ChangedFile { path: new, old_path: Some(old), status, additions: None, deletions: None });
        } else {
            let path = fields.next().unwrap_or("").to_string();
            files.push(ChangedFile { path, old_path: None, status, additions: None, deletions: None });
        }
    }
    files
}

pub fn apply_numstat(files: &mut [ChangedFile], bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    let mut fields = text.split('\0').peekable();
    while let Some(rec) = fields.next() {
        if rec.is_empty() {
            continue;
        }
        let mut parts = rec.splitn(3, '\t');
        let add = parts.next().and_then(|s| s.parse::<u32>().ok());
        let del = parts.next().and_then(|s| s.parse::<u32>().ok());
        let mut path = parts.next().unwrap_or("").to_string();
        if path.is_empty() {
            // Rename: old and new paths follow as separate fields.
            let _old = fields.next();
            path = fields.next().unwrap_or("").to_string();
        }
        if let Some(f) = files.iter_mut().find(|f| f.path == path) {
            f.additions = add;
            f.deletions = del;
        }
    }
}

// -------------------------------------------------------------------------
// Unified diff
// -------------------------------------------------------------------------

pub fn parse_diff(bytes: &[u8]) -> Vec<FileDiff> {
    let text = String::from_utf8_lossy(bytes);
    let mut diffs: Vec<FileDiff> = Vec::new();
    let mut current: Option<FileDiff> = None;
    let mut old_no = 0u32;
    let mut new_no = 0u32;
    let mut in_hunk = false;

    for raw in text.split('\n') {
        if raw.starts_with("diff --git ") || raw.starts_with("diff --cc ") || raw.starts_with("diff --combined ") {
            if let Some(d) = current.take() {
                diffs.push(d);
            }
            let mut d = FileDiff::default();
            if let Some((a, b)) = split_diff_git_paths(&raw["diff --git ".len().min(raw.len())..]) {
                d.path = b;
                if a != d.path {
                    d.old_path = Some(a);
                }
            }
            d.header.push(raw.to_string());
            current = Some(d);
            in_hunk = false;
            continue;
        }
        let Some(d) = current.as_mut() else { continue };
        if raw.starts_with("@@") {
            in_hunk = true;
            let (os, oc, ns, nc) = parse_hunk_header(raw);
            old_no = os;
            new_no = ns;
            d.hunks.push(Hunk { header: raw.to_string(), old_start: os, old_count: oc, new_start: ns, new_count: nc, lines: Vec::new() });
            continue;
        }
        if !in_hunk {
            if let Some(p) = raw.strip_prefix("+++ ") {
                if p != "/dev/null" {
                    d.path = unquote(p.strip_prefix("b/").unwrap_or(p));
                }
            } else if let Some(p) = raw.strip_prefix("--- ") {
                if p != "/dev/null" {
                    let old = unquote(p.strip_prefix("a/").unwrap_or(p));
                    if old != d.path && !d.path.is_empty() {
                        d.old_path = Some(old);
                    } else {
                        d.old_path = None;
                    }
                }
            } else if raw.starts_with("new file mode") {
                d.new_file = true;
            } else if raw.starts_with("deleted file mode") {
                d.deleted = true;
            } else if raw.starts_with("Binary files") || raw.starts_with("GIT binary patch") {
                d.binary = true;
            } else if let Some(p) = raw.strip_prefix("rename to ") {
                d.path = unquote(p);
            } else if let Some(p) = raw.strip_prefix("rename from ") {
                d.old_path = Some(unquote(p));
            }
            if !raw.is_empty() {
                d.header.push(raw.to_string());
            }
            continue;
        }
        let Some(hunk) = d.hunks.last_mut() else { continue };
        if raw.starts_with('\\') {
            if let Some(last) = hunk.lines.last_mut() {
                last.no_newline = true;
            }
            continue;
        }
        let (kind, content) = match raw.as_bytes().first() {
            Some(b'+') => (LineKind::Added, &raw[1..]),
            Some(b'-') => (LineKind::Removed, &raw[1..]),
            Some(b' ') => (LineKind::Context, &raw[1..]),
            None => {
                // Trailing empty string after the final newline, or an
                // empty context line produced by some diff tools.
                continue;
            }
            _ => continue,
        };
        let line = match kind {
            LineKind::Added => {
                new_no += 1;
                DiffLine { kind, content: content.to_string(), old_no: None, new_no: Some(new_no - 1), no_newline: false }
            }
            LineKind::Removed => {
                old_no += 1;
                DiffLine { kind, content: content.to_string(), old_no: Some(old_no - 1), new_no: None, no_newline: false }
            }
            LineKind::Context => {
                old_no += 1;
                new_no += 1;
                DiffLine { kind, content: content.to_string(), old_no: Some(old_no - 1), new_no: Some(new_no - 1), no_newline: false }
            }
        };
        hunk.lines.push(line);
    }
    if let Some(d) = current.take() {
        diffs.push(d);
    }
    diffs
}

fn split_diff_git_paths(s: &str) -> Option<(String, String)> {
    // "a/path b/path" — paths may contain spaces; both halves are equal for
    // non-renames, so split in the middle when possible.
    let s = s.trim();
    if let (Some(a), true) = (s.strip_prefix("a/"), s.contains(" b/")) {
        let bytes = s.len();
        // Try the symmetric split first.
        if bytes % 2 == 1 {
            let mid = bytes / 2;
            let (left, right) = s.split_at(mid);
            if let (Some(l), Some(r)) = (left.strip_prefix("a/"), right.strip_prefix(" b/")) {
                if l == r {
                    return Some((l.to_string(), r.to_string()));
                }
            }
        }
        let idx = a.find(" b/")?;
        return Some((unquote(&a[..idx]), unquote(&a[idx + 3..])));
    }
    None
}

fn unquote(s: &str) -> String {
    let s = s.trim_end_matches('\t');
    if s.len() < 2 || !s.starts_with('"') || !s.ends_with('"') {
        return s.to_string();
    }
    let inner = &s.as_bytes()[1..s.len() - 1];
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    let is_octal = |b: u8| (b'0'..=b'7').contains(&b);
    while i < inner.len() {
        if inner[i] != b'\\' || i + 1 >= inner.len() {
            out.push(inner[i]);
            i += 1;
            continue;
        }
        if i + 4 <= inner.len() && is_octal(inner[i + 1]) && is_octal(inner[i + 2]) && is_octal(inner[i + 3]) {
            let v = (inner[i + 1] - b'0') as u32 * 64 + (inner[i + 2] - b'0') as u32 * 8 + (inner[i + 3] - b'0') as u32;
            out.push(v as u8);
            i += 4;
            continue;
        }
        out.push(match inner[i + 1] {
            b'n' => b'\n',
            b't' => b'\t',
            other => other,
        });
        i += 2;
    }
    let text = String::from_utf8_lossy(&out).into_owned();
    text.strip_prefix("a/").or_else(|| text.strip_prefix("b/")).map(String::from).unwrap_or(text)
}

fn parse_hunk_header(h: &str) -> (u32, u32, u32, u32) {
    // @@ -a,b +c,d @@ context
    let mut old = (0, 1);
    let mut new = (0, 1);
    for part in h.split_whitespace().skip(1) {
        if part.starts_with("@@") {
            break;
        }
        if let Some(p) = part.strip_prefix('-') {
            old = parse_range(p);
        } else if let Some(p) = part.strip_prefix('+') {
            new = parse_range(p);
        }
    }
    (old.0, old.1, new.0, new.1)
}

fn parse_range(spec: &str) -> (u32, u32) {
    match spec.split_once(',') {
        Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
        None => (spec.parse().unwrap_or(0), 1),
    }
}

/// Presents a file containing conflict markers: our side as removed lines,
/// their side as added lines, everything else (and the markers) as context.
pub fn conflict_view(path: &str, bytes: &[u8]) -> FileDiff {
    let mut d = FileDiff { path: path.to_string(), conflict: true, ..Default::default() };
    if bytes[..bytes.len().min(8000)].contains(&0) {
        d.binary = true;
        return d;
    }
    let text = String::from_utf8_lossy(bytes);
    let body = text.strip_suffix('\n').unwrap_or(&text);
    #[derive(PartialEq)]
    enum Side {
        None,
        Ours,
        Base,
        Theirs,
    }
    let mut side = Side::None;
    let mut lines = Vec::new();
    for (i, l) in body.split('\n').enumerate() {
        let n = Some(i as u32 + 1);
        let trimmed = l.trim_end_matches('\r');
        let kind = if trimmed.starts_with("<<<<<<<") {
            side = Side::Ours;
            LineKind::Context
        } else if trimmed.starts_with("|||||||") && side == Side::Ours {
            side = Side::Base;
            LineKind::Context
        } else if trimmed.starts_with("=======") && side != Side::None {
            side = Side::Theirs;
            LineKind::Context
        } else if trimmed.starts_with(">>>>>>>") && side != Side::None {
            side = Side::None;
            LineKind::Context
        } else {
            match side {
                Side::Ours => LineKind::Removed,
                Side::Theirs => LineKind::Added,
                Side::Base | Side::None => LineKind::Context,
            }
        };
        lines.push(DiffLine { kind, content: l.to_string(), old_no: None, new_no: n, no_newline: false });
    }
    let n = lines.len() as u32;
    d.hunks.push(Hunk { header: format!("@@ -1,{n} +1,{n} @@ conflict"), old_start: 1, old_count: n, new_start: 1, new_count: n, lines });
    d
}

/// Builds a diff that adds every line of `bytes` as a new file.
pub fn synthesize_added_diff(path: &str, bytes: &[u8]) -> FileDiff {
    let mut d = FileDiff { path: path.to_string(), new_file: true, ..Default::default() };
    d.header = vec![format!("diff --git a/{path} b/{path}"), "new file mode 100644".into(), "--- /dev/null".into(), format!("+++ b/{path}")];
    let probe = &bytes[..bytes.len().min(8000)];
    if probe.contains(&0) {
        d.binary = true;
        return d;
    }
    if bytes.is_empty() {
        return d;
    }
    let text = String::from_utf8_lossy(bytes);
    let ends_with_newline = text.ends_with('\n');
    let body = text.strip_suffix('\n').unwrap_or(&text);
    let mut lines: Vec<DiffLine> = body
        .split('\n')
        .enumerate()
        .map(|(i, l)| DiffLine { kind: LineKind::Added, content: l.to_string(), old_no: None, new_no: Some(i as u32 + 1), no_newline: false })
        .collect();
    if !ends_with_newline {
        if let Some(last) = lines.last_mut() {
            last.no_newline = true;
        }
    }
    let n = lines.len() as u32;
    d.hunks.push(Hunk { header: format!("@@ -0,0 +1,{n} @@"), old_start: 0, old_count: 0, new_start: 1, new_count: n, lines });
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parsing() {
        let raw = b"# branch.oid 1234567890abcdef1234567890abcdef12345678\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\01 M. N... 100644 100644 100644 aaa bbb src/lib.rs\01 .M N... 100644 100644 100644 aaa bbb README.md\02 R. N... 100644 100644 100644 aaa bbb R100 new name.rs\0old.rs\0u UU N... 100644 100644 100644 100644 a b c conflict.txt\0? untracked file.txt\0";
        let (head, st) = parse_status(raw);
        assert_eq!(head.branch.as_deref(), Some("main"));
        assert_eq!(head.upstream.as_deref(), Some("origin/main"));
        assert_eq!((head.ahead, head.behind), (2, 1));
        assert_eq!(st.staged.len(), 2);
        let renamed = st.staged.iter().find(|e| e.status == FileStatus::Renamed).unwrap();
        assert_eq!(renamed.path, "new name.rs");
        assert_eq!(renamed.old_path.as_deref(), Some("old.rs"));
        assert_eq!(st.unstaged.len(), 3);
        assert_eq!(st.unstaged[0].status, FileStatus::Conflicted);
        assert_eq!(st.unstaged[2].path, "untracked file.txt");
    }

    #[test]
    fn diff_parsing() {
        let raw = b"diff --git a/foo.txt b/foo.txt\nindex 1..2 100644\n--- a/foo.txt\n+++ b/foo.txt\n@@ -1,3 +1,3 @@ fn main\n a\n-b\n+c\n d\n\\ No newline at end of file\n";
        let d = &parse_diff(raw)[0];
        assert_eq!(d.path, "foo.txt");
        assert_eq!(d.hunks.len(), 1);
        let h = &d.hunks[0];
        assert_eq!((h.old_start, h.old_count, h.new_start, h.new_count), (1, 3, 1, 3));
        assert_eq!(h.lines.len(), 4);
        assert_eq!(h.lines[1].kind, LineKind::Removed);
        assert_eq!(h.lines[2].new_no, Some(2));
        assert!(h.lines[3].no_newline);
        assert_eq!(h.context(), "fn main");
    }

    #[test]
    fn diff_paths_with_spaces() {
        let raw = b"diff --git a/my file.txt b/my file.txt\nnew file mode 100644\n--- /dev/null\n+++ b/my file.txt\n@@ -0,0 +1 @@\n+hi\n";
        let d = &parse_diff(raw)[0];
        assert_eq!(d.path, "my file.txt");
        assert!(d.new_file);
        assert_eq!(d.hunks[0].new_count, 1);
    }

    #[test]
    fn refs_parsing() {
        let raw = "refs/heads/main\0abc\0\0origin/main\0ahead 1, behind 2\0*\0100\0subject\0\nrefs/remotes/origin/HEAD\0abc\0\0\0\0 \0100\0subject\0refs/remotes/origin/main\nrefs/remotes/origin/feature/x\0def\0\0\0\0 \0100\0s\0\nrefs/tags/v1\0tagobj\0abc\0\0\0 \0100\0Release\0\n";
        let refs = parse_refs(raw);
        assert_eq!(refs.branches.len(), 1);
        assert_eq!(refs.branches[0].ahead, 1);
        assert_eq!(refs.branches[0].behind, 2);
        assert!(refs.branches[0].is_head);
        assert_eq!(refs.remote_branches.len(), 1);
        assert_eq!(refs.remote_branches[0].name, "feature/x");
        assert_eq!(refs.tags[0].oid, "abc");
        assert!(refs.tags[0].annotated);
    }
}
