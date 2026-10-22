//! The host editors' rules, without GPUI: what each dialog validates, which
//! row it writes, and which secret it stores. Ports of `HostModal.vue`,
//! `MongoDbModal.vue`, `RedisModal.vue`, `VncModal.vue` and
//! `utils/hostOptions.js`, message for message.
//!
//! Three rules hold across every kind:
//!
//! - **An edit writes the whole row.** `update_host` rewrites every column,
//!   so a record is built over the row it edits and changes only what the
//!   form shows. Leaving a column out is how editing a host used to ungroup
//!   it in the Vue app; here nothing the form does not show can change.
//! - **A secret is never shown and never cleared by an empty field.** The
//!   password fields start empty. An empty one on save means "keep what is
//!   stored" ([`secret_to_store`]); only a typed value is written.
//! - **A URI never carries its password into the row.** The password is
//!   split out (`termdrop_core::uri::split_password`) and stored as a secret,
//!   exactly as the Vue dialogs' save handlers did.

use std::collections::BTreeMap;

use termdrop_core::uri::{expects_password, split_password, with_password};

use super::host_kind::HostKind;
use super::uri_form::{
    MongoFields, RedisFields, build_mongo_uri, build_redis_uri, encode_uri_component, is_mongo_uri,
    is_redis_uri, is_srv_like_mongo_uri, is_tls_redis_uri, parse_vnc_target,
};
use crate::backend::{HostRecord, HostSummary, SecretKind};

/// A field an error can be shown under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    Name,
    Host,
    Port,
    Username,
    Password,
    KeyPath,
    Uri,
    /// A VNC address, on a VNC host or in an SSH host's remote desktop part.
    Target,
    /// The name typed for a new group.
    NewGroup,
}

pub type Errors = BTreeMap<Field, String>;

/// Which dialog edits a row. Chosen by what the row is, as `editHost` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorKind {
    Ssh,
    Mongo,
    Redis,
    Vnc,
}

impl EditorKind {
    pub fn for_host(kind: HostKind) -> Self {
        match kind {
            HostKind::Mongodb => Self::Mongo,
            HostKind::Redis => Self::Redis,
            HostKind::Vnc => Self::Vnc,
            HostKind::Ssh | HostKind::Local => Self::Ssh,
        }
    }

    /// The dialog title, in each Vue dialog's own wording.
    pub fn title(self, editing: bool) -> &'static str {
        match (self, editing) {
            (Self::Ssh, true) => "Edit Host",
            (Self::Ssh, false) => "Add Host",
            (Self::Mongo, true) => "Edit MongoDB connection",
            (Self::Mongo, false) => "Add MongoDB connection",
            (Self::Redis, true) => "Edit Redis connection",
            (Self::Redis, false) => "Add Redis connection",
            (Self::Vnc, true) => "Edit remote desktop",
            (Self::Vnc, false) => "Add remote desktop",
        }
    }

    pub fn save_label(self, editing: bool) -> &'static str {
        match (self, editing) {
            (Self::Ssh, true) => "Save Changes",
            (Self::Ssh, false) => "Add Host",
            (Self::Redis, _) => "Save",
            (_, true) => "Save",
            (_, false) => "Add",
        }
    }

    /// The toast when the row itself could not be saved.
    pub fn save_failed(self, err: &str) -> String {
        match self {
            // The Vue SSH dialog let this reach the global handler; say what
            // failed instead.
            Self::Ssh => format!("Failed to save host: {err}"),
            Self::Mongo => format!("Failed to save MongoDB connection: {err}"),
            Self::Redis => format!("Failed to save Redis connection: {err}"),
            Self::Vnc => format!("Failed to save the remote desktop: {err}"),
        }
    }

    pub fn name_placeholder(self) -> &'static str {
        match self {
            Self::Ssh => "My Server",
            Self::Mongo => "Staging DB",
            Self::Redis => "Production cache",
            Self::Vnc => "Kiosk screen",
        }
    }

    /// The name error. The datastore dialogs were written later and say it
    /// differently; each keeps its own.
    fn name_required(self) -> &'static str {
        match self {
            Self::Ssh | Self::Mongo => "Name is required",
            Self::Redis | Self::Vnc => "A name is required",
        }
    }
}

/// The value to store from a password field, or `None` to keep what is
/// stored. Only a typed value is ever written; an empty field never clears.
pub fn secret_to_store(typed: &str) -> Option<String> {
    (!typed.is_empty()).then(|| typed.to_string())
}

/// The port checks every dialog shares.
pub fn port_error(raw: &str) -> Option<&'static str> {
    let t = raw.trim();
    if t.is_empty() {
        return Some("Port is required");
    }
    match t.parse::<f64>() {
        Ok(n) if n.is_finite() && n.fract() == 0.0 => {
            (!(1.0..=65535.0).contains(&n)).then_some("Port must be 1–65535")
        }
        _ => Some("Port must be an integer"),
    }
}

fn blank(s: &str) -> bool {
    s.trim().is_empty()
}

// ---- what every editor shares -------------------------------------------

/// The fields every editor has: name, group and favourite.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Common {
    pub name: String,
    pub group: GroupChoice,
    /// What is typed for a new group, when `group` is [`GroupChoice::New`].
    pub new_group: String,
    pub favorite: bool,
}

/// The group picker's value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum GroupChoice {
    /// A group by name; the empty name is Ungrouped.
    Existing(String),
    /// "New group…": the name is typed beside it.
    New,
    #[default]
    Ungrouped,
}

impl Common {
    pub fn from_record(rec: &HostRecord) -> Self {
        Self {
            name: rec.name.clone(),
            group: match rec.group.as_deref() {
                None | Some("") => GroupChoice::Ungrouped,
                Some(g) => GroupChoice::Existing(g.to_string()),
            },
            new_group: String::new(),
            favorite: rec.favorite,
        }
    }

    /// A new host, preselected into `group` ("Add host here").
    pub fn new_in(group: Option<&str>) -> Self {
        Self {
            group: match group {
                None | Some("") => GroupChoice::Ungrouped,
                Some(g) => GroupChoice::Existing(g.to_string()),
            },
            ..Default::default()
        }
    }

    fn validate(&self, kind: EditorKind, errors: &mut Errors) {
        if blank(&self.name) {
            errors.insert(Field::Name, kind.name_required().into());
        }
        if self.group == GroupChoice::New && blank(&self.new_group) {
            errors.insert(Field::NewGroup, "Group name is required".into());
        }
    }

    /// The `group` column: `None` for ungrouped, as the Vue dialogs sent.
    pub fn group_column(&self) -> Option<String> {
        match &self.group {
            GroupChoice::Ungrouped => None,
            GroupChoice::Existing(g) if g.is_empty() => None,
            GroupChoice::Existing(g) => Some(g.clone()),
            GroupChoice::New => Some(self.new_group.trim().to_string()),
        }
    }

    fn apply(&self, rec: &mut HostRecord) {
        rec.name = self.name.trim().to_string();
        rec.group = self.group_column();
        rec.favorite = self.favorite;
    }
}

/// The group picker's entries: Ungrouped, every group in play, then "New
/// group…". `group_names` is `host_grouping::group_names`.
pub fn group_choices(group_names: &[String]) -> Vec<(GroupChoice, String)> {
    let mut out = vec![(GroupChoice::Ungrouped, "Ungrouped".to_string())];
    out.extend(
        group_names
            .iter()
            .filter(|g| !g.is_empty())
            .map(|g| (GroupChoice::Existing(g.clone()), g.clone())),
    );
    out.push((GroupChoice::New, "New group…".to_string()));
    out
}

/// A row to start an add from: the columns every Vue add dialog sent.
fn blank_record() -> HostRecord {
    HostRecord {
        auth_type: "password".into(),
        ..Default::default()
    }
}

/// What a save writes: the row, and the secrets to store once it has an id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavePlan {
    pub record: HostRecord,
    pub secrets: Vec<(SecretKind, String)>,
}

// ---- SSH -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthType {
    #[default]
    Password,
    Key,
}

impl AuthType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Key => "key",
        }
    }
}

/// An SSH host's remote desktop: the VNC fields an SSH row can carry. An
/// empty address means the host has none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VncPart {
    pub target: String,
    pub password: String,
    pub username: String,
    pub tunnel_host_id: Option<i64>,
    pub view_only: bool,
}

impl VncPart {
    fn from_record(rec: &HostRecord) -> Self {
        Self {
            target: super::display::vnc_display_target(rec.vnc_host.as_deref(), rec.vnc_port),
            password: String::new(),
            username: rec.vnc_username.clone().unwrap_or_default(),
            tunnel_host_id: rec.vnc_tunnel_host_id,
            view_only: rec.vnc_view_only,
        }
    }

    /// Write the VNC columns. `Err` carries the address error.
    fn apply(&self, rec: &mut HostRecord) -> Result<Option<String>, String> {
        let (host, port) = parse_vnc_target(&self.target)?;
        rec.vnc_host = Some(host);
        rec.vnc_port = Some(port);
        rec.vnc_username = Some(self.username.trim().to_string()).filter(|u| !u.is_empty());
        rec.vnc_tunnel_host_id = self.tunnel_host_id;
        rec.vnc_view_only = self.view_only;
        Ok(secret_to_store(&self.password))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshForm {
    pub common: Common,
    pub host: String,
    pub port: String,
    pub username: String,
    pub auth: AuthType,
    pub key_path: String,
    pub password: String,
    pub jump_host_id: Option<i64>,
    /// Off until the user opens the remote desktop part or the row has one.
    pub vnc_enabled: bool,
    pub vnc: VncPart,
}

impl Default for SshForm {
    fn default() -> Self {
        Self {
            common: Common::default(),
            host: String::new(),
            port: "22".into(),
            username: String::new(),
            auth: AuthType::Password,
            key_path: String::new(),
            password: String::new(),
            jump_host_id: None,
            vnc_enabled: false,
            vnc: VncPart::default(),
        }
    }
}

impl SshForm {
    /// `resetForm` for an edit. The password stays empty.
    pub fn from_record(rec: &HostRecord) -> Self {
        let has_vnc = rec.vnc_host.as_deref().is_some_and(|h| !h.is_empty());
        Self {
            common: Common::from_record(rec),
            host: rec.host.clone(),
            port: if rec.port == 0 {
                "22".into()
            } else {
                rec.port.to_string()
            },
            username: rec.username.clone(),
            auth: if rec.auth_type == "key" {
                AuthType::Key
            } else {
                AuthType::Password
            },
            key_path: rec.key_path.clone().unwrap_or_default(),
            password: String::new(),
            jump_host_id: rec.jump_host_id,
            vnc_enabled: has_vnc,
            vnc: if has_vnc {
                VncPart::from_record(rec)
            } else {
                VncPart::default()
            },
        }
    }

    pub fn validate(&self, editing: bool) -> Errors {
        let mut e = Errors::new();
        self.common.validate(EditorKind::Ssh, &mut e);
        if blank(&self.host) {
            e.insert(Field::Host, "Host is required".into());
        }
        if let Some(msg) = port_error(&self.port) {
            e.insert(Field::Port, msg.into());
        }
        if blank(&self.username) {
            e.insert(Field::Username, "Username is required".into());
        }
        match self.auth {
            AuthType::Password if !editing && blank(&self.password) => {
                e.insert(Field::Password, "Password is required for new hosts".into());
            }
            AuthType::Key if blank(&self.key_path) => {
                e.insert(Field::KeyPath, "Key path is required".into());
            }
            _ => {}
        }
        if self.vnc_enabled
            && let Err(msg) = parse_vnc_target(&self.vnc.target)
        {
            e.insert(Field::Target, msg);
        }
        e
    }

    /// The row and secrets to write, over `existing` when editing. Call only
    /// once [`Self::validate`] is empty.
    pub fn plan(&self, existing: Option<&HostRecord>) -> Result<SavePlan, Errors> {
        let errors = self.validate(existing.is_some());
        if !errors.is_empty() {
            return Err(errors);
        }
        let mut rec = existing.cloned().unwrap_or_else(blank_record);
        self.common.apply(&mut rec);
        rec.host = self.host.trim().to_string();
        rec.port = self.port.trim().parse::<f64>().map_or(22, |n| n as u16);
        rec.username = self.username.trim().to_string();
        rec.auth_type = self.auth.as_str().into();
        rec.key_path = match self.auth {
            AuthType::Key => Some(self.key_path.trim().to_string()),
            AuthType::Password => None,
        };
        rec.jump_host_id = self.jump_host_id;
        // A MongoDB connection is its own host; this column is legacy.
        rec.mongo_local_uri = None;

        let mut secrets = Vec::new();
        if self.auth == AuthType::Password
            && let Some(pw) = secret_to_store(&self.password)
        {
            secrets.push((SecretKind::SshPassword, pw));
        }
        if self.vnc_enabled {
            match self.vnc.apply(&mut rec) {
                Ok(Some(pw)) => secrets.push((SecretKind::Vnc, pw)),
                Ok(None) => {}
                Err(msg) => return Err(Errors::from([(Field::Target, msg)])),
            }
        } else {
            rec.vnc_host = None;
            rec.vnc_port = None;
            rec.vnc_username = None;
            rec.vnc_tunnel_host_id = None;
            rec.vnc_view_only = false;
        }
        Ok(SavePlan {
            record: rec,
            secrets,
        })
    }
}

// ---- MongoDB -------------------------------------------------------------

/// The Mongo dialog. `uri` is the full connection string the form stands
/// for, password included when one was typed; the box shows it through
/// [`masked_uri`]. Typing in the box replaces `uri`; typing in a field
/// rebuilds it, as `rebuildUri` did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MongoForm {
    pub common: Common,
    pub uri: String,
    pub fields: MongoFields,
}

impl MongoForm {
    /// The dialog for an existing row. The stored URI's own password, if a
    /// row still has one, is neither shown nor dropped: see [`Self::plan`].
    pub fn from_record(rec: &HostRecord) -> Self {
        let stored = rec.mongo_uri.clone().unwrap_or_default();
        let (stripped, _) = split_password(&stored);
        let (uri, fields) = super::uri_form::mongo_uri_for_edit(&stripped);
        Self {
            common: Common::from_record(rec),
            uri,
            fields: fields.unwrap_or_default(),
        }
    }

    pub fn is_srv(&self) -> bool {
        is_srv_like_mongo_uri(&self.uri)
    }

    /// The URI box was typed in: fill the fields from it when it parses.
    ///
    /// The box never shows the password, so a URI typed there usually has
    /// none; a password already in the field is then kept rather than wiped
    /// by an edit to some other part of the URI.
    pub fn uri_typed(&mut self, typed: &str) {
        let kept = std::mem::take(&mut self.fields.password);
        self.uri = typed.to_string();
        if let Some(f) = super::uri_form::parse_mongo_uri_to_form(typed) {
            self.fields = f;
        }
        if self.fields.password.is_empty() && !kept.is_empty() && expects_password(typed) {
            self.uri = with_password(typed, &encode_uri_component(&kept));
            self.fields.password = kept;
        }
    }

    /// A field was typed in: rebuild the URI from the fields.
    pub fn fields_changed(&mut self) {
        self.uri = build_mongo_uri(&self.fields);
    }

    pub fn validate(&self) -> Errors {
        let mut e = Errors::new();
        self.common.validate(EditorKind::Mongo, &mut e);
        let uri = self.uri.trim();
        if uri.is_empty() {
            e.insert(Field::Uri, "Connection string is required".into());
        } else if !is_mongo_uri(uri) {
            e.insert(
                Field::Uri,
                "URI must start with mongodb:// or mongodb+srv://".into(),
            );
        }
        if !self.is_srv() {
            if blank(&self.fields.host) {
                e.insert(Field::Host, "Host is required".into());
            }
            if let Some(msg) = port_error(&self.fields.port) {
                e.insert(Field::Port, msg.into());
            }
        }
        e
    }

    pub fn plan(&self, existing: Option<&HostRecord>) -> Result<SavePlan, Errors> {
        let errors = self.validate();
        if !errors.is_empty() {
            return Err(errors);
        }
        let (uri, typed) = split_password(self.uri.trim());
        let legacy = existing
            .and_then(|r| r.mongo_uri.as_deref())
            .and_then(|u| split_password(u).1);
        let mut rec = existing.cloned().unwrap_or_else(blank_record);
        self.common.apply(&mut rec);
        rec.host = String::new();
        rec.port = 0;
        rec.username = String::new();
        rec.auth_type = "password".into();
        rec.key_path = None;
        rec.mongo_uri = Some(uri);
        rec.mongo_local_uri = None;
        Ok(SavePlan {
            record: rec,
            secrets: keep_or_store(SecretKind::Mongo, typed, legacy),
        })
    }
}

/// The secret a URI dialog stores: one typed into the URI or the password
/// field (which the URI already carries by then), else a password the row
/// still had inside its stored URI, moved to the keyring rather than lost.
/// Nothing at all keeps whatever the keyring has.
fn keep_or_store(
    kind: SecretKind,
    typed: Option<String>,
    legacy: Option<String>,
) -> Vec<(SecretKind, String)> {
    typed
        .or(legacy)
        .filter(|s| !s.is_empty())
        .map(|s| vec![(kind, s)])
        .unwrap_or_default()
}

/// What the URI box shows for a URI: the same text without its password.
pub fn masked_uri(uri: &str) -> String {
    split_password(uri).0
}

// ---- Redis ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RedisForm {
    pub common: Common,
    pub uri: String,
    pub fields: RedisFields,
    pub tunnel_host_id: Option<i64>,
}

impl RedisForm {
    pub fn from_record(rec: &HostRecord) -> Self {
        let (stripped, _) = split_password(rec.redis_uri.as_deref().unwrap_or_default());
        let mut form = Self {
            common: Common::from_record(rec),
            tunnel_host_id: rec.redis_tunnel_host_id,
            ..Default::default()
        };
        form.uri_typed(&stripped);
        form
    }

    /// As [`MongoForm::uri_typed`], including keeping a typed password.
    pub fn uri_typed(&mut self, typed: &str) {
        let kept = std::mem::take(&mut self.fields.password);
        self.uri = typed.to_string();
        self.fields = super::uri_form::parse_redis_uri(typed);
        if self.fields.password.is_empty() && !kept.is_empty() && expects_password(typed) {
            self.uri = with_password(typed, &kept);
            self.fields.password = kept;
        }
    }

    pub fn fields_changed(&mut self) {
        self.uri = build_redis_uri(&self.fields);
    }

    pub fn validate(&self) -> Errors {
        let mut e = Errors::new();
        self.common.validate(EditorKind::Redis, &mut e);
        if blank(&self.uri) {
            e.insert(Field::Uri, "A connection URI is required".into());
        } else if !is_redis_uri(&self.uri) {
            e.insert(Field::Uri, "Must start with redis:// or rediss://".into());
        } else if is_tls_redis_uri(&self.uri) && self.tunnel_host_id.is_some() {
            e.insert(
                Field::Uri,
                "A TLS connection cannot be tunnelled — turn TLS off, or connect directly".into(),
            );
        }
        e
    }

    pub fn plan(&self, existing: Option<&HostRecord>) -> Result<SavePlan, Errors> {
        let errors = self.validate();
        if !errors.is_empty() {
            return Err(errors);
        }
        let (uri, typed) = split_password(self.uri.trim());
        let legacy = existing
            .and_then(|r| r.redis_uri.as_deref())
            .and_then(|u| split_password(u).1);
        let mut rec = existing.cloned().unwrap_or_else(blank_record);
        self.common.apply(&mut rec);
        rec.host = String::new();
        rec.port = 0;
        rec.username = String::new();
        rec.auth_type = "password".into();
        rec.key_path = None;
        rec.mongo_uri = None;
        rec.mongo_local_uri = None;
        rec.redis_uri = Some(uri);
        rec.redis_tunnel_host_id = self.tunnel_host_id;
        Ok(SavePlan {
            record: rec,
            secrets: keep_or_store(SecretKind::Redis, typed, legacy),
        })
    }
}

// ---- VNC -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VncForm {
    pub common: Common,
    pub vnc: VncPart,
}

impl VncForm {
    pub fn from_record(rec: &HostRecord) -> Self {
        Self {
            common: Common::from_record(rec),
            vnc: VncPart::from_record(rec),
        }
    }

    pub fn validate(&self) -> Errors {
        let mut e = Errors::new();
        self.common.validate(EditorKind::Vnc, &mut e);
        if let Err(msg) = parse_vnc_target(&self.vnc.target) {
            e.insert(Field::Target, msg);
        }
        e
    }

    pub fn plan(&self, existing: Option<&HostRecord>) -> Result<SavePlan, Errors> {
        let errors = self.validate();
        if !errors.is_empty() {
            return Err(errors);
        }
        let mut rec = existing.cloned().unwrap_or_else(blank_record);
        self.common.apply(&mut rec);
        rec.host = String::new();
        rec.port = 0;
        rec.username = String::new();
        rec.auth_type = "password".into();
        rec.key_path = None;
        rec.mongo_uri = None;
        rec.mongo_local_uri = None;
        rec.redis_uri = None;
        rec.redis_tunnel_host_id = None;
        let secret = self
            .vnc
            .apply(&mut rec)
            .map_err(|msg| Errors::from([(Field::Target, msg)]))?;
        Ok(SavePlan {
            record: rec,
            secrets: secret
                .map(|s| vec![(SecretKind::Vnc, s)])
                .unwrap_or_default(),
        })
    }
}

// ---- pickers -------------------------------------------------------------

/// One entry of a "go through this SSH host" picker. `id: None` is the
/// direct route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostChoice {
    pub id: Option<i64>,
    pub label: String,
    pub hint: Option<String>,
}

fn ssh_choice(h: &HostSummary) -> HostChoice {
    HostChoice {
        id: Some(h.id),
        label: h.name.clone(),
        hint: Some(format!("{}@{}", h.username, h.host)),
    }
}

/// `sshHostOptions`: "Connect directly", then each host with its address as a
/// hint, so two hosts sharing a name can still be told apart.
pub fn ssh_host_choices<'a>(hosts: impl IntoIterator<Item = &'a HostSummary>) -> Vec<HostChoice> {
    let mut out = vec![HostChoice {
        id: None,
        label: "Connect directly".into(),
        hint: None,
    }];
    out.extend(hosts.into_iter().map(ssh_choice));
    out
}

/// `jumpHostCandidates`: SSH hosts only, never the host itself, and never one
/// with a jump host of its own -- one hop is all the backend supports.
pub fn jump_host_candidates(hosts: &[HostSummary], self_id: Option<i64>) -> Vec<&HostSummary> {
    hosts
        .iter()
        .filter(|h| h.kind == HostKind::Ssh && Some(h.id) != self_id && h.jump_host_id.is_none())
        .collect()
}

/// The SSH hosts a Redis or VNC connection can be tunnelled through.
pub fn tunnel_hosts(hosts: &[HostSummary], self_id: Option<i64>) -> Vec<&HostSummary> {
    hosts
        .iter()
        .filter(|h| h.kind == HostKind::Ssh && Some(h.id) != self_id)
        .collect()
}

/// A saved jump host whose row is gone. Shown rather than silently dropped,
/// so saving the form is what changes the route, not opening it.
pub fn jump_missing(jump_host_id: Option<i64>, hosts: &[HostSummary]) -> bool {
    jump_host_id.is_some_and(|id| !hosts.iter().any(|h| h.id == id))
}

/// The VNC tunnel picker. On an SSH host the direct entry is *this host's*
/// connection, because a VNC server there almost always binds 127.0.0.1.
pub fn vnc_tunnel_choices(tunnels: &[&HostSummary], is_ssh_host: bool) -> Vec<HostChoice> {
    let mut out = vec![HostChoice {
        id: None,
        label: if is_ssh_host {
            "This host’s SSH connection".into()
        } else {
            "Directly".into()
        },
        hint: is_ssh_host.then(|| "recommended".to_string()),
    }];
    out.extend(tunnels.iter().map(|h| ssh_choice(h)));
    out
}

pub fn vnc_tunnel_hint(tunnel_host_id: Option<i64>, is_ssh_host: bool) -> &'static str {
    match (tunnel_host_id, is_ssh_host) {
        (Some(_), _) => "The address above is resolved on that host.",
        (None, true) => "The address above is resolved on this host — usually 127.0.0.1.",
        (None, false) => "The VNC server must be reachable from this machine.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_form() -> SshForm {
        SshForm {
            common: Common {
                name: " web ".into(),
                ..Default::default()
            },
            host: " 10.0.0.1 ".into(),
            username: "root".into(),
            password: "hunter2".into(),
            ..Default::default()
        }
    }

    fn existing() -> HostRecord {
        HostRecord {
            id: 7,
            name: "web".into(),
            host: "10.0.0.1".into(),
            port: 2222,
            username: "root".into(),
            auth_type: "password".into(),
            group: Some("Prod".into()),
            favorite: true,
            redis_uri: Some("redis://side.cache".into()),
            last_connected_at: Some("2026-10-01 10:00:00".into()),
            ..Default::default()
        }
    }

    #[test]
    fn ssh_validation_uses_the_vue_messages() {
        let e = SshForm::default().validate(false);
        assert_eq!(e[&Field::Name], "Name is required");
        assert_eq!(e[&Field::Host], "Host is required");
        assert_eq!(e[&Field::Username], "Username is required");
        assert_eq!(e[&Field::Password], "Password is required for new hosts");
        assert!(!e.contains_key(&Field::Port), "22 is the default");

        // Editing keeps the stored password, so none is required.
        assert!(
            !SshForm::default()
                .validate(true)
                .contains_key(&Field::Password)
        );

        let key = SshForm {
            auth: AuthType::Key,
            ..ssh_form()
        };
        assert_eq!(key.validate(false)[&Field::KeyPath], "Key path is required");
    }

    #[test]
    fn ports_are_checked_as_the_vue_form_did() {
        assert_eq!(port_error(""), Some("Port is required"));
        assert_eq!(port_error("abc"), Some("Port must be an integer"));
        assert_eq!(port_error("22.5"), Some("Port must be an integer"));
        assert_eq!(port_error("0"), Some("Port must be 1–65535"));
        assert_eq!(port_error("70000"), Some("Port must be 1–65535"));
        assert_eq!(port_error(" 2222 "), None);
    }

    #[test]
    fn a_new_ssh_host_is_built_from_the_form() {
        let plan = SshForm {
            common: Common {
                group: GroupChoice::New,
                new_group: " Lab ".into(),
                favorite: true,
                ..ssh_form().common
            },
            jump_host_id: Some(3),
            ..ssh_form()
        }
        .plan(None)
        .unwrap();
        let r = &plan.record;
        assert_eq!(
            (r.name.as_str(), r.host.as_str(), r.port),
            ("web", "10.0.0.1", 22)
        );
        assert_eq!(r.auth_type, "password");
        assert_eq!(r.key_path, None);
        assert_eq!(r.group.as_deref(), Some("Lab"));
        assert!(r.favorite);
        assert_eq!(r.jump_host_id, Some(3));
        assert_eq!(plan.secrets, [(SecretKind::SshPassword, "hunter2".into())]);
    }

    #[test]
    fn an_edit_keeps_every_column_the_form_does_not_show() {
        let rec = existing();
        let mut form = SshForm::from_record(&rec);
        assert_eq!(form.password, "", "a secret is never prefilled");
        assert_eq!(form.port, "2222");
        form.common.name = "web-renamed".into();
        let plan = form.plan(Some(&rec)).unwrap();
        let r = &plan.record;
        assert_eq!(r.id, 7);
        assert_eq!(r.name, "web-renamed");
        assert_eq!(r.group.as_deref(), Some("Prod"), "editing must not ungroup");
        assert!(r.favorite, "editing must not drop the star");
        assert_eq!(r.redis_uri.as_deref(), Some("redis://side.cache"));
        assert!(
            plan.secrets.is_empty(),
            "an empty field keeps the stored password"
        );
    }

    #[test]
    fn switching_to_a_key_drops_the_password_but_never_stores_one() {
        let plan = SshForm {
            auth: AuthType::Key,
            key_path: " ~/.ssh/id ".into(),
            ..ssh_form()
        }
        .plan(None)
        .unwrap();
        assert_eq!(plan.record.auth_type, "key");
        assert_eq!(plan.record.key_path.as_deref(), Some("~/.ssh/id"));
        assert!(plan.secrets.is_empty());
    }

    #[test]
    fn an_ssh_host_can_carry_a_remote_desktop() {
        let mut form = ssh_form();
        form.vnc_enabled = true;
        form.vnc.target = "127.0.0.1:1".into();
        form.vnc.password = "vncpw".into();
        form.vnc.view_only = true;
        let plan = form.plan(None).unwrap();
        assert_eq!(plan.record.vnc_host.as_deref(), Some("127.0.0.1"));
        assert_eq!(plan.record.vnc_port, Some(5901));
        assert!(plan.record.vnc_view_only);
        assert!(plan.secrets.contains(&(SecretKind::Vnc, "vncpw".into())));

        form.vnc.target = "fe80::1:5901".into();
        assert!(form.validate(false).contains_key(&Field::Target));

        // Turning it off clears the columns.
        let rec = HostRecord {
            vnc_host: Some("127.0.0.1".into()),
            vnc_port: Some(5901),
            ..existing()
        };
        let mut form = SshForm::from_record(&rec);
        assert!(form.vnc_enabled);
        assert_eq!(form.vnc.target, "127.0.0.1:5901");
        form.vnc_enabled = false;
        let plan = form.plan(Some(&rec)).unwrap();
        assert_eq!((plan.record.vnc_host, plan.record.vnc_port), (None, None));
    }

    #[test]
    fn a_mongo_password_is_split_out_of_the_uri() {
        let mut form = MongoForm {
            common: Common {
                name: "orders".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        form.uri_typed("mongodb://admin:s3cret@db.internal/orders");
        assert_eq!(form.fields.password, "s3cret");
        let plan = form.plan(None).unwrap();
        assert_eq!(
            plan.record.mongo_uri.as_deref(),
            Some("mongodb://admin@db.internal/orders")
        );
        assert_eq!(plan.record.host, "");
        assert_eq!(plan.secrets, [(SecretKind::Mongo, "s3cret".into())]);
        assert_eq!(masked_uri(&form.uri), "mongodb://admin@db.internal/orders");
    }

    #[test]
    fn a_mongo_password_typed_in_its_field_is_stored_encoded() {
        let mut form = MongoForm::default();
        form.common.name = "x".into();
        form.fields.host = "h".into();
        form.fields.username = "u".into();
        form.fields.password = "p@ss".into();
        form.fields_changed();
        let plan = form.plan(None).unwrap();
        assert_eq!(plan.record.mongo_uri.as_deref(), Some("mongodb://u@h"));
        // Encoded, as the Vue form stored it, so splicing it back is exact.
        assert_eq!(plan.secrets, [(SecretKind::Mongo, "p%40ss".into())]);
    }

    #[test]
    fn editing_the_uri_box_keeps_a_password_typed_in_its_field() {
        let mut form = MongoForm::default();
        form.common.name = "x".into();
        form.fields.host = "h".into();
        form.fields.username = "u".into();
        form.fields.password = "pw".into();
        form.fields_changed();
        // The box shows the URI without the password; the user edits that.
        let shown = masked_uri(&form.uri);
        assert_eq!(shown, "mongodb://u@h");
        form.uri_typed(&format!("{shown}/orders"));
        assert_eq!(form.fields.password, "pw");
        assert_eq!(form.fields.database, "orders");
        let plan = form.plan(None).unwrap();
        assert_eq!(
            plan.record.mongo_uri.as_deref(),
            Some("mongodb://u@h/orders")
        );
        assert_eq!(plan.secrets, [(SecretKind::Mongo, "pw".into())]);

        let mut redis = RedisForm::default();
        redis.fields.host = "h".into();
        redis.fields.password = "pw".into();
        redis.fields_changed();
        redis.uri_typed(&format!("{}/2", masked_uri(&redis.uri)));
        assert_eq!(redis.uri, "redis://:pw@h/2");
        // With no `@` left there is nowhere for it to go, and it is dropped.
        redis.uri_typed("redis://h");
        assert_eq!(redis.fields.password, "");
    }

    #[test]
    fn editing_a_mongo_host_never_shows_or_clears_its_password() {
        let rec = HostRecord {
            id: 3,
            name: "orders".into(),
            mongo_uri: Some("mongodb://admin@db.internal:27018/orders".into()),
            ..Default::default()
        };
        let form = MongoForm::from_record(&rec);
        assert_eq!(form.fields.password, "");
        assert_eq!(form.uri, "mongodb://admin@db.internal:27018/orders");
        let plan = form.plan(Some(&rec)).unwrap();
        assert!(plan.secrets.is_empty(), "empty means keep");

        // A legacy row with the password still in its URI: hidden in the
        // form, and moved to the keyring on save rather than lost.
        let legacy = HostRecord {
            mongo_uri: Some("mongodb://admin:old@db.internal/orders".into()),
            ..rec
        };
        let form = MongoForm::from_record(&legacy);
        assert!(!form.uri.contains("old"));
        let plan = form.plan(Some(&legacy)).unwrap();
        assert_eq!(
            plan.record.mongo_uri.as_deref(),
            Some("mongodb://admin@db.internal/orders")
        );
        assert_eq!(plan.secrets, [(SecretKind::Mongo, "old".into())]);
    }

    #[test]
    fn mongo_validation_relaxes_for_srv() {
        let mut form = MongoForm::default();
        assert_eq!(
            form.validate()[&Field::Uri],
            "Connection string is required"
        );
        form.uri_typed("postgres://x");
        assert_eq!(
            form.validate()[&Field::Uri],
            "URI must start with mongodb:// or mongodb+srv://"
        );
        form.uri_typed("mongodb+srv://cluster0.example.net");
        form.fields.host.clear();
        let e = form.validate();
        assert!(!e.contains_key(&Field::Host) && !e.contains_key(&Field::Port));
    }

    #[test]
    fn a_redis_password_is_split_out_and_tls_cannot_be_tunnelled() {
        let mut form = RedisForm::default();
        form.common.name = "cache".into();
        form.fields.host = "cache.internal".into();
        form.fields.password = "pw".into();
        form.fields_changed();
        assert_eq!(form.uri, "redis://:pw@cache.internal");
        let plan = form.plan(None).unwrap();
        // `redis://@host` still expects a password at connect time.
        assert_eq!(
            plan.record.redis_uri.as_deref(),
            Some("redis://@cache.internal")
        );
        assert_eq!(plan.secrets, [(SecretKind::Redis, "pw".into())]);

        form.fields.tls = true;
        form.fields_changed();
        form.tunnel_host_id = Some(1);
        assert_eq!(
            form.validate()[&Field::Uri],
            "A TLS connection cannot be tunnelled — turn TLS off, or connect directly"
        );
        assert_eq!(
            RedisForm::default().validate()[&Field::Name],
            "A name is required"
        );
    }

    #[test]
    fn a_vnc_host_is_built_from_its_address() {
        let form = VncForm {
            common: Common {
                name: "kiosk".into(),
                ..Default::default()
            },
            vnc: VncPart {
                target: "10.0.0.5:1".into(),
                username: "  ".into(),
                ..Default::default()
            },
        };
        let plan = form.plan(None).unwrap();
        assert_eq!(plan.record.vnc_host.as_deref(), Some("10.0.0.5"));
        assert_eq!(plan.record.vnc_port, Some(5901));
        assert_eq!(plan.record.vnc_username, None);
        assert!(plan.secrets.is_empty());
        assert_eq!(
            VncForm::default().validate()[&Field::Target],
            "An address is required"
        );
    }

    #[test]
    fn the_secret_keep_rule() {
        assert_eq!(secret_to_store(""), None);
        assert_eq!(secret_to_store(" x "), Some(" x ".into()), "never trimmed");
    }

    #[test]
    fn group_choices_and_the_new_group_rule() {
        let names = vec![String::new(), "Lab".into(), "Prod".into()];
        let choices = group_choices(&names);
        assert_eq!(choices.first().unwrap().1, "Ungrouped");
        assert_eq!(choices.last().unwrap().0, GroupChoice::New);
        assert_eq!(choices.len(), 4, "the empty name is Ungrouped, listed once");

        let mut c = Common::new_in(Some("Lab"));
        assert_eq!(c.group_column().as_deref(), Some("Lab"));
        c.group = GroupChoice::New;
        let mut e = Errors::new();
        c.name = "x".into();
        c.validate(EditorKind::Ssh, &mut e);
        assert_eq!(e[&Field::NewGroup], "Group name is required");
        assert_eq!(Common::new_in(Some("")).group_column(), None);
    }

    fn summary(id: i64, kind_host: &str, jump: Option<i64>) -> HostSummary {
        HostRecord {
            id,
            name: format!("h{id}"),
            host: kind_host.into(),
            username: "u".into(),
            jump_host_id: jump,
            redis_uri: kind_host.is_empty().then(|| "redis://x".to_string()),
            ..Default::default()
        }
        .summary()
    }

    #[test]
    fn jump_hosts_exclude_self_datastores_and_chained_hosts() {
        let hosts = vec![
            summary(1, "a", None),
            summary(2, "b", Some(1)),
            summary(3, "", None),
            summary(4, "d", None),
        ];
        let ids: Vec<_> = jump_host_candidates(&hosts, Some(4))
            .iter()
            .map(|h| h.id)
            .collect();
        assert_eq!(ids, [1]);
        let choices = ssh_host_choices(jump_host_candidates(&hosts, None));
        assert_eq!(choices[0].label, "Connect directly");
        assert_eq!(choices[1].hint.as_deref(), Some("u@a"));
        assert!(jump_missing(Some(9), &hosts));
        assert!(!jump_missing(Some(1), &hosts));
        assert!(!jump_missing(None, &hosts));
    }

    #[test]
    fn vnc_tunnel_choices_depend_on_the_row() {
        let hosts = vec![summary(1, "a", None)];
        let t = tunnel_hosts(&hosts, None);
        assert_eq!(vnc_tunnel_choices(&t, false)[0].label, "Directly");
        let ssh = vnc_tunnel_choices(&t, true);
        assert_eq!(ssh[0].label, "This host’s SSH connection");
        assert_eq!(ssh[0].hint.as_deref(), Some("recommended"));
        assert_eq!(
            vnc_tunnel_hint(None, false),
            "The VNC server must be reachable from this machine."
        );
    }

    #[test]
    fn titles_and_labels_follow_each_dialog() {
        assert_eq!(EditorKind::Ssh.title(false), "Add Host");
        assert_eq!(EditorKind::Ssh.save_label(true), "Save Changes");
        assert_eq!(EditorKind::Mongo.save_label(false), "Add");
        assert_eq!(EditorKind::Redis.save_label(false), "Save");
        assert_eq!(EditorKind::Vnc.title(true), "Edit remote desktop");
        assert_eq!(EditorKind::for_host(HostKind::Redis), EditorKind::Redis);
    }
}
