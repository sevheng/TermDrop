//! The Tunnels panel's and the forward dialog's pure parts: the form, its
//! validation (`PortForwardModal.vue`'s `validateField` / `validateAll`),
//! what a save sends, and the labels `PortForwardPanel.vue` draws.

use std::collections::BTreeMap;

use termdrop_core::db::{NewPortForward, PortForward};

pub const DEFAULT_LOCAL_HOST: &str = "127.0.0.1";
pub const DEFAULT_REMOTE_HOST: &str = "localhost";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForwardKind {
    /// `-L`: a local port to a host and port reached from the server.
    #[default]
    Local,
    /// `-D`: a SOCKS5 proxy on the local address.
    Dynamic,
}

impl ForwardKind {
    /// The `kind` column (`CHECK(kind IN ('local', 'dynamic'))`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Dynamic => "dynamic",
        }
    }

    /// Vue's `editForward.kind || 'local'`: anything but "dynamic" is local.
    pub fn parse(kind: &str) -> Self {
        if kind == "dynamic" {
            Self::Dynamic
        } else {
            Self::Local
        }
    }
}

/// What the dialog opens with for a new forward: the Vue `prefill` prop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefill {
    pub name: String,
    pub kind: ForwardKind,
    pub local_host: String,
    pub local_port: Option<u32>,
    pub remote_host: String,
    pub remote_port: Option<u32>,
}

impl Prefill {
    /// `createForwardFromSelection` in `TerminalTab.vue`: the same port on
    /// both ends, named after it.
    pub fn forward_port(port: u32) -> Self {
        Self {
            name: format!("Forward {port}"),
            kind: ForwardKind::Local,
            local_host: DEFAULT_LOCAL_HOST.into(),
            local_port: Some(port),
            remote_host: DEFAULT_REMOTE_HOST.into(),
            remote_port: Some(port),
        }
    }
}

/// The form's text, as typed. Ports stay strings until validated, as the
/// Vue number inputs held whatever was typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardForm {
    pub name: String,
    pub kind: ForwardKind,
    pub local_host: String,
    pub local_port: String,
    pub remote_host: String,
    pub remote_port: String,
}

impl Default for ForwardForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: ForwardKind::Local,
            local_host: DEFAULT_LOCAL_HOST.into(),
            local_port: String::new(),
            remote_host: DEFAULT_REMOTE_HOST.into(),
            remote_port: String::new(),
        }
    }
}

fn or_default(value: &str, default: &str) -> String {
    if value.is_empty() {
        default.into()
    } else {
        value.into()
    }
}

fn port_text(port: Option<impl ToString>) -> String {
    port.map(|p| p.to_string()).unwrap_or_default()
}

impl ForwardForm {
    pub fn from_prefill(p: &Prefill) -> Self {
        Self {
            name: p.name.clone(),
            kind: p.kind,
            local_host: or_default(&p.local_host, DEFAULT_LOCAL_HOST),
            local_port: port_text(p.local_port),
            remote_host: or_default(&p.remote_host, DEFAULT_REMOTE_HOST),
            remote_port: port_text(p.remote_port),
        }
    }

    pub fn from_forward(f: &PortForward) -> Self {
        Self {
            name: f.name.clone(),
            kind: ForwardKind::parse(&f.kind),
            local_host: or_default(&f.local_host, DEFAULT_LOCAL_HOST),
            local_port: port_text(Some(f.local_port)),
            remote_host: or_default(f.remote_host.as_deref().unwrap_or(""), DEFAULT_REMOTE_HOST),
            remote_port: port_text(f.remote_port),
        }
    }

    /// The row `onSave` built. Call only after [`validate_all`] passed.
    pub fn to_new(&self, host_id: i64) -> NewPortForward {
        let local = self.kind == ForwardKind::Local;
        let local_host = self.local_host.trim();
        NewPortForward {
            host_id,
            name: self.name.trim().to_string(),
            kind: self.kind.as_str().to_string(),
            local_host: if local_host.is_empty() {
                DEFAULT_LOCAL_HOST.to_string()
            } else {
                local_host.to_string()
            },
            local_port: port_value(&self.local_port).unwrap_or(0),
            remote_host: local.then(|| self.remote_host.trim().to_string()),
            remote_port: local.then(|| port_value(&self.remote_port).unwrap_or(0)),
        }
    }
}

/// A port as typed, if it is a whole number.
fn port_value(text: &str) -> Option<i64> {
    text.trim().parse::<f64>().ok().map(|n| n as i64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    Name,
    LocalPort,
    RemoteHost,
    RemotePort,
}

pub const PORT_RANGE: &str = "Port must be 1\u{2013}65535";

/// What a port field holds, the way `v-model.number` on a number input
/// saw it: nothing (also what the browser reports for text that is not a
/// number), zero (falsy, so "required" too), or a number.
fn port_problem(text: &str, required: &'static str) -> Option<&'static str> {
    let text = text.trim();
    let Ok(n) = text.parse::<f64>() else {
        return Some(required);
    };
    if n == 0.0 || n.is_nan() {
        Some(required)
    } else if !(1.0..=65535.0).contains(&n) || n.fract() != 0.0 {
        // A fraction passed the Vue check and then failed to save; here it
        // is refused with the range message instead.
        Some(PORT_RANGE)
    } else {
        None
    }
}

/// `validateField(field)`: the message for one field, or `None`.
pub fn validate_field(form: &ForwardForm, field: Field) -> Option<&'static str> {
    let local = form.kind == ForwardKind::Local;
    match field {
        Field::Name => form.name.trim().is_empty().then_some("Name is required"),
        Field::LocalPort => port_problem(&form.local_port, "Local port is required"),
        Field::RemoteHost => {
            (local && form.remote_host.trim().is_empty()).then_some("Remote host is required")
        }
        Field::RemotePort => {
            if local {
                port_problem(&form.remote_port, "Remote port is required")
            } else {
                None
            }
        }
    }
}

/// Apply [`validate_field`] to `errors`, as a blur does.
pub fn check_field(form: &ForwardForm, field: Field, errors: &mut BTreeMap<Field, &'static str>) {
    match validate_field(form, field) {
        Some(msg) => {
            errors.insert(field, msg);
        }
        None => {
            errors.remove(&field);
        }
    }
}

/// `validateAll()`: true when the form can be saved. A dynamic forward has
/// no remote fields, so their errors are dropped rather than left to block
/// the save unseen (the Vue modal kept them, and a remote-port error made
/// before switching to Dynamic could not be cleared).
pub fn validate_all(form: &ForwardForm, errors: &mut BTreeMap<Field, &'static str>) -> bool {
    check_field(form, Field::Name, errors);
    check_field(form, Field::LocalPort, errors);
    if form.kind == ForwardKind::Local {
        check_field(form, Field::RemoteHost, errors);
        check_field(form, Field::RemotePort, errors);
    } else {
        errors.remove(&Field::RemoteHost);
        errors.remove(&Field::RemotePort);
    }
    errors.is_empty()
}

pub fn dialog_title(editing: bool) -> &'static str {
    if editing {
        "Edit Port Forward"
    } else {
        "Add Port Forward"
    }
}

pub fn save_label(saving: bool, editing: bool) -> &'static str {
    match (saving, editing) {
        (true, _) => "Saving...",
        (false, true) => "Save Changes",
        (false, false) => "Add Forward",
    }
}

pub fn saved_message(editing: bool) -> &'static str {
    if editing {
        "Port forward updated"
    } else {
        "Port forward added"
    }
}

/// The SOCKS hint's address: `{{ form.local_host }}:{{ form.local_port }}`,
/// exactly as typed.
pub fn socks_address(form: &ForwardForm) -> String {
    format!("{}:{}", form.local_host, form.local_port)
}

/// The Dynamic hint as one sentence, and where the address sits in it.
pub fn socks_hint(form: &ForwardForm) -> (String, std::ops::Range<usize>) {
    let lead = "Dynamic forwarding creates a SOCKS5 proxy on the local address. Configure your \
                browser or app to use ";
    let address = socks_address(form);
    let range = lead.len()..lead.len() + address.len();
    (format!("{lead}{address} as a SOCKS5 proxy."), range)
}

// ---- the panel -----------------------------------------------------------

/// The header's count: `forwards.length || ''`.
pub fn count_meta(n: usize) -> String {
    if n == 0 { String::new() } else { n.to_string() }
}

pub fn is_local(f: &PortForward) -> bool {
    f.kind == "local"
}

/// The first summary line: "Local → 127.0.0.1:8080" / "SOCKS → ...".
pub fn listen_line(f: &PortForward) -> String {
    let kind = if is_local(f) { "Local" } else { "SOCKS" };
    format!("{kind} \u{2192} {}:{}", f.local_host, f.local_port)
}

/// The second line, for a local forward only: "localhost:80".
pub fn target_line(f: &PortForward) -> Option<String> {
    is_local(f).then(|| {
        format!(
            "{}:{}",
            f.remote_host.as_deref().unwrap_or(""),
            port_text(f.remote_port)
        )
    })
}

/// "Open in browser" is offered for a running local forward only.
pub fn can_open(f: &PortForward, active: bool) -> bool {
    active && is_local(f)
}

pub fn browser_url(f: &PortForward) -> String {
    format!("http://{}:{}", f.local_host, f.local_port)
}

pub fn status_label(active: bool) -> &'static str {
    if active { "Active" } else { "Stopped" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(kind: &str) -> PortForward {
        PortForward {
            id: 1,
            host_id: 5,
            name: "Web".into(),
            kind: kind.into(),
            local_host: "127.0.0.1".into(),
            local_port: 8080,
            remote_host: (kind == "local").then(|| "localhost".into()),
            remote_port: (kind == "local").then_some(80),
        }
    }

    fn form() -> ForwardForm {
        ForwardForm {
            name: "MySQL".into(),
            local_port: "13306".into(),
            remote_port: "3306".into(),
            ..ForwardForm::default()
        }
    }

    #[test]
    fn a_new_form_has_the_vue_defaults() {
        let f = ForwardForm::default();
        assert_eq!(f.kind, ForwardKind::Local);
        assert_eq!(f.local_host, "127.0.0.1");
        assert_eq!(f.remote_host, "localhost");
        assert_eq!(f.local_port, "");
        assert_eq!(f.remote_port, "");
    }

    #[test]
    fn editing_fills_in_blank_hosts() {
        let mut fw = saved("dynamic");
        fw.local_host = String::new();
        let f = ForwardForm::from_forward(&fw);
        assert_eq!(f.kind, ForwardKind::Dynamic);
        assert_eq!(f.local_host, "127.0.0.1");
        assert_eq!(f.remote_host, "localhost");
        assert_eq!(f.remote_port, "");
        assert_eq!(f.local_port, "8080");
    }

    #[test]
    fn field_messages_are_the_vue_ones() {
        let mut f = ForwardForm::default();
        assert_eq!(validate_field(&f, Field::Name), Some("Name is required"));
        f.name = "   ".into();
        assert_eq!(validate_field(&f, Field::Name), Some("Name is required"));
        assert_eq!(
            validate_field(&f, Field::LocalPort),
            Some("Local port is required")
        );
        f.local_port = "0".into();
        assert_eq!(
            validate_field(&f, Field::LocalPort),
            Some("Local port is required")
        );
        f.local_port = "abc".into();
        assert_eq!(
            validate_field(&f, Field::LocalPort),
            Some("Local port is required")
        );
        f.local_port = "70000".into();
        assert_eq!(
            validate_field(&f, Field::LocalPort),
            Some("Port must be 1–65535")
        );
        f.local_port = "-5".into();
        assert_eq!(validate_field(&f, Field::LocalPort), Some(PORT_RANGE));
        f.local_port = "65535".into();
        assert_eq!(validate_field(&f, Field::LocalPort), None);

        f.remote_host = " ".into();
        assert_eq!(
            validate_field(&f, Field::RemoteHost),
            Some("Remote host is required")
        );
        assert_eq!(
            validate_field(&f, Field::RemotePort),
            Some("Remote port is required")
        );
        f.remote_port = "99999".into();
        assert_eq!(validate_field(&f, Field::RemotePort), Some(PORT_RANGE));

        f.kind = ForwardKind::Dynamic;
        assert_eq!(validate_field(&f, Field::RemoteHost), None);
        assert_eq!(validate_field(&f, Field::RemotePort), None);
    }

    #[test]
    fn validate_all_checks_remote_fields_only_for_local() {
        let mut errors = BTreeMap::new();
        let mut f = form();
        f.remote_port = String::new();
        assert!(!validate_all(&f, &mut errors));
        assert_eq!(
            errors.get(&Field::RemotePort),
            Some(&"Remote port is required")
        );

        f.kind = ForwardKind::Dynamic;
        assert!(validate_all(&f, &mut errors));
        assert!(errors.is_empty());
    }

    #[test]
    fn a_blur_sets_and_clears_one_error() {
        let mut errors = BTreeMap::new();
        let mut f = ForwardForm::default();
        check_field(&f, Field::Name, &mut errors);
        assert_eq!(errors.len(), 1);
        f.name = "x".into();
        check_field(&f, Field::Name, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn a_save_sends_the_onsave_row() {
        let mut f = form();
        f.name = "  MySQL  ".into();
        f.local_host = "  ".into();
        f.remote_host = " db.internal ".into();
        let row = f.to_new(9);
        assert_eq!(row.host_id, 9);
        assert_eq!(row.name, "MySQL");
        assert_eq!(row.kind, "local");
        assert_eq!(row.local_host, "127.0.0.1");
        assert_eq!(row.local_port, 13306);
        assert_eq!(row.remote_host.as_deref(), Some("db.internal"));
        assert_eq!(row.remote_port, Some(3306));

        f.kind = ForwardKind::Dynamic;
        let row = f.to_new(9);
        assert_eq!(row.kind, "dynamic");
        assert_eq!(row.remote_host, None);
        assert_eq!(row.remote_port, None);
    }

    #[test]
    fn dialog_wording() {
        assert_eq!(dialog_title(false), "Add Port Forward");
        assert_eq!(dialog_title(true), "Edit Port Forward");
        assert_eq!(save_label(false, false), "Add Forward");
        assert_eq!(save_label(false, true), "Save Changes");
        assert_eq!(save_label(true, true), "Saving...");
        assert_eq!(saved_message(false), "Port forward added");
        assert_eq!(saved_message(true), "Port forward updated");
        assert_eq!(socks_address(&form()), "127.0.0.1:13306");
        let (text, at) = socks_hint(&form());
        assert_eq!(&text[at], "127.0.0.1:13306");
        assert!(text.starts_with("Dynamic forwarding creates a SOCKS5 proxy on the local address. Configure your browser or app to use 127.0.0.1:13306 as a SOCKS5 proxy."));
    }

    #[test]
    fn prefill_fills_the_form() {
        let f = ForwardForm::from_prefill(&Prefill::forward_port(3000));
        assert_eq!(f.name, "Forward 3000");
        assert_eq!(f.local_port, "3000");
        assert_eq!(f.remote_port, "3000");
        assert_eq!(f.remote_host, "localhost");
    }

    #[test]
    fn panel_lines() {
        let local = saved("local");
        assert_eq!(listen_line(&local), "Local → 127.0.0.1:8080");
        assert_eq!(target_line(&local).as_deref(), Some("localhost:80"));
        assert_eq!(browser_url(&local), "http://127.0.0.1:8080");
        assert!(can_open(&local, true));
        assert!(!can_open(&local, false));

        let socks = saved("dynamic");
        assert_eq!(listen_line(&socks), "SOCKS → 127.0.0.1:8080");
        assert_eq!(target_line(&socks), None);
        assert!(!can_open(&socks, true));

        assert_eq!(status_label(true), "Active");
        assert_eq!(status_label(false), "Stopped");
        assert_eq!(count_meta(0), "");
        assert_eq!(count_meta(3), "3");
    }
}
