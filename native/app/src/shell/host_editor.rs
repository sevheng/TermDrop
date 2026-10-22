//! The four host editors as one dialog view. Ports of `HostModal.vue`,
//! `MongoDbModal.vue` + `MongoConnectionFields.vue`, `RedisModal.vue` +
//! `RedisConnectionFields.vue` and `VncModal.vue` + `VncConnectionFields.vue`:
//! the same fields, defaults, placeholders and messages.
//!
//! The rules (validation, which row is written, which secret is stored) are
//! in `app::host_form`; this file lays the fields out and keeps the inputs in
//! step with the form. Three things it adds to the Vue dialogs, all of which
//! the editors had no room for before: a group picker (with a new group), a
//! favourite box, and an SSH host's own remote desktop settings.
//!
//! Secrets are masked inputs that start empty, always. Whether one is stored
//! is shown in the placeholder and hint, never by filling the field.

use std::cell::Cell;
use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, FontWeight, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Window, div, px,
};

use crate::app::AppModel;
use crate::app::host_form::{
    AuthType, Common, EditorKind, Errors, Field, GroupChoice, MongoForm, RedisForm, SaveOutcome,
    SavePlan, SshForm, VncForm, group_choices, jump_host_candidates, jump_missing, masked_uri,
    run_save, ssh_host_choices, tunnel_hosts, vnc_tunnel_choices, vnc_tunnel_hint,
};
use crate::backend::{HostRecord, HostSummary, SecretKind};
use crate::ui::choice::{Choice, index_of};
use crate::ui::file_dialog;
use crate::ui::theme::{Palette, palette};
use crate::ui::toast::{ToastKind, toast};

/// The text inputs an editor may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum In {
    Name,
    NewGroup,
    Host,
    Port,
    Username,
    Password,
    KeyPath,
    Uri,
    Database,
    AuthSource,
    Options,
    Target,
    VncPassword,
    VncUsername,
}

enum Form {
    Ssh(SshForm),
    Mongo(MongoForm),
    Redis(RedisForm),
    Vnc(VncForm),
}

impl Form {
    fn common(&self) -> &Common {
        match self {
            Self::Ssh(f) => &f.common,
            Self::Mongo(f) => &f.common,
            Self::Redis(f) => &f.common,
            Self::Vnc(f) => &f.common,
        }
    }

    fn common_mut(&mut self) -> &mut Common {
        match self {
            Self::Ssh(f) => &mut f.common,
            Self::Mongo(f) => &mut f.common,
            Self::Redis(f) => &mut f.common,
            Self::Vnc(f) => &mut f.common,
        }
    }

    fn plan(&self, existing: Option<&HostRecord>) -> Result<SavePlan, Errors> {
        match self {
            Self::Ssh(f) => f.plan(existing),
            Self::Mongo(f) => f.plan(existing),
            Self::Redis(f) => f.plan(existing),
            Self::Vnc(f) => f.plan(existing),
        }
    }
}

type RouteSelect = Entity<SelectState<Vec<Choice<Option<i64>>>>>;
type GroupSelect = Entity<SelectState<Vec<Choice<GroupChoice>>>>;

pub struct HostEditor {
    kind: EditorKind,
    model: Entity<AppModel>,
    existing: Option<HostRecord>,
    form: Form,
    inputs: HashMap<In, Entity<InputState>>,
    group_select: GroupSelect,
    /// SSH: the jump host. Redis and VNC: the tunnel host.
    route_select: Option<RouteSelect>,
    /// SSH only: the tunnel for the host's own remote desktop.
    vnc_route_select: Option<RouteSelect>,
    hosts: Vec<HostSummary>,
    errors: Errors,
    /// The kind's own secret is stored (Mongo, Redis, VNC), or for an SSH
    /// host, its remote desktop's.
    has_secret: bool,
    /// Set once a save succeeded, so a second Confirm cannot save twice or
    /// close a dialog beneath this one.
    done: bool,
    _subscriptions: Vec<Subscription>,
}

/// The toast when a secret could not be stored, after the row was saved.
fn secret_failed(kind: SecretKind, err: &str) -> String {
    match kind {
        SecretKind::SshPassword => format!("Password could not be saved: {err}"),
        SecretKind::Mongo => EditorKind::Mongo.save_failed(err),
        SecretKind::Redis => EditorKind::Redis.save_failed(err),
        SecretKind::Vnc => EditorKind::Vnc.save_failed(err),
    }
}

impl HostEditor {
    fn new(
        kind: EditorKind,
        model: Entity<AppModel>,
        existing: Option<HostRecord>,
        preset_group: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editing = existing.is_some();
        let self_id = existing.as_ref().map(|r| r.id);
        let form = match (&existing, kind) {
            (Some(r), EditorKind::Ssh) => Form::Ssh(SshForm::from_record(r)),
            (Some(r), EditorKind::Mongo) => Form::Mongo(MongoForm::from_record(r)),
            (Some(r), EditorKind::Redis) => Form::Redis(RedisForm::from_record(r)),
            (Some(r), EditorKind::Vnc) => Form::Vnc(VncForm::from_record(r)),
            (None, k) => {
                let common = Common::new_in(preset_group.as_deref());
                match k {
                    EditorKind::Ssh => Form::Ssh(SshForm {
                        common,
                        ..Default::default()
                    }),
                    EditorKind::Mongo => Form::Mongo(MongoForm {
                        common,
                        ..Default::default()
                    }),
                    EditorKind::Redis => Form::Redis(RedisForm {
                        common,
                        ..Default::default()
                    }),
                    EditorKind::Vnc => Form::Vnc(VncForm {
                        common,
                        ..Default::default()
                    }),
                }
            }
        };

        let (hosts, group_names, has_secret) = {
            let m = model.read(cx);
            let secret = match (self_id, kind) {
                (Some(id), EditorKind::Mongo) => m.has_secret(SecretKind::Mongo, id),
                (Some(id), EditorKind::Redis) => m.has_secret(SecretKind::Redis, id),
                (Some(id), EditorKind::Vnc | EditorKind::Ssh) => m.has_secret(SecretKind::Vnc, id),
                (None, _) => false,
            };
            (m.hosts().to_vec(), m.group_names(), secret)
        };

        // ---- inputs ------------------------------------------------------
        let mut specs: Vec<(In, String, &str, bool)> = vec![
            (
                In::Name,
                form.common().name.clone(),
                kind.name_placeholder(),
                false,
            ),
            (In::NewGroup, String::new(), "e.g. Production", false),
        ];
        let stored = "•••••••• (stored)";
        match &form {
            Form::Ssh(f) => specs.extend([
                (
                    In::Host,
                    f.host.clone(),
                    "192.168.1.1 or example.com",
                    false,
                ),
                (In::Port, f.port.clone(), "22", false),
                (In::Username, f.username.clone(), "root", false),
                (
                    In::Password,
                    String::new(),
                    if editing {
                        "Leave empty to keep existing"
                    } else {
                        ""
                    },
                    true,
                ),
                (In::KeyPath, f.key_path.clone(), "~/.ssh/id_rsa", false),
                (In::Target, f.vnc.target.clone(), "127.0.0.1:1", false),
                (
                    In::VncPassword,
                    String::new(),
                    if has_secret {
                        "Leave blank to keep"
                    } else {
                        ""
                    },
                    true,
                ),
                (In::VncUsername, f.vnc.username.clone(), "", false),
            ]),
            Form::Mongo(f) => specs.extend([
                (
                    In::Uri,
                    masked_uri(&f.uri),
                    "mongodb:// or mongodb+srv://",
                    false,
                ),
                (In::Host, f.fields.host.clone(), "host or IP", false),
                (In::Port, f.fields.port.clone(), "27017", false),
                (In::Username, f.fields.username.clone(), "user", false),
                (
                    In::Password,
                    String::new(),
                    if has_secret { stored } else { "password" },
                    true,
                ),
                (In::Database, f.fields.database.clone(), "database", false),
                (In::AuthSource, f.fields.auth_source.clone(), "admin", false),
                (
                    In::Options,
                    f.fields.options.clone(),
                    "retryWrites=true&replicaSet=rs0",
                    false,
                ),
            ]),
            Form::Redis(f) => specs.extend([
                (
                    In::Uri,
                    masked_uri(&f.uri),
                    "redis://:password@localhost:6379/0",
                    false,
                ),
                (In::Host, f.fields.host.clone(), "localhost", false),
                (In::Port, f.fields.port.clone(), "6379", false),
                (In::Database, f.fields.database.clone(), "0", false),
                (In::Username, f.fields.username.clone(), "", false),
                (
                    In::Password,
                    String::new(),
                    if has_secret { stored } else { "" },
                    true,
                ),
            ]),
            Form::Vnc(f) => specs.extend([
                (In::Target, f.vnc.target.clone(), "10.0.0.5:1", false),
                (
                    In::VncPassword,
                    String::new(),
                    if has_secret {
                        "Leave blank to keep"
                    } else {
                        ""
                    },
                    true,
                ),
                (In::VncUsername, f.vnc.username.clone(), "", false),
            ]),
        }

        let mut inputs = HashMap::new();
        let mut subscriptions = Vec::new();
        for (which, value, placeholder, masked) in specs {
            let input = cx.new(|cx| {
                let mut s = InputState::new(window, cx)
                    .placeholder(placeholder)
                    .masked(masked);
                if !value.is_empty() {
                    s = s.default_value(value);
                }
                s
            });
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        this.on_input(which, value, window, cx);
                    }
                },
            ));
            inputs.insert(which, input);
        }

        // ---- pickers -----------------------------------------------------
        let groups: Vec<Choice<GroupChoice>> = group_choices(&group_names)
            .into_iter()
            .map(|(v, l)| Choice::new(v, l))
            .collect();
        let group_ix = index_of(&groups, &form.common().group).or(Some(0));
        let group_select =
            cx.new(|cx| SelectState::new(groups, group_ix.map(IndexPath::new), window, cx));
        subscriptions.push(cx.subscribe_in(
            &group_select,
            window,
            |this, _, event: &SelectEvent<Vec<Choice<GroupChoice>>>, window, cx| {
                let SelectEvent::Confirm(Some(choice)) = event else {
                    return;
                };
                this.form.common_mut().group = choice.clone();
                this.errors.remove(&Field::NewGroup);
                if *choice == GroupChoice::New
                    && let Some(input) = this.inputs.get(&In::NewGroup)
                {
                    input.focus_handle(cx).focus(window, cx);
                }
                cx.notify();
            },
        ));

        let to_choices =
            |list: Vec<crate::app::host_form::HostChoice>| -> Vec<Choice<Option<i64>>> {
                list.into_iter()
                    .map(|c| Choice::new(c.id, c.label).hint(c.hint))
                    .collect()
            };
        let route_choices = match &form {
            Form::Ssh(_) => Some(to_choices(ssh_host_choices(jump_host_candidates(
                &hosts, self_id,
            )))),
            Form::Redis(_) => Some(to_choices(ssh_host_choices(tunnel_hosts(&hosts, self_id)))),
            Form::Vnc(_) => Some(to_choices(vnc_tunnel_choices(
                &tunnel_hosts(&hosts, self_id),
                false,
            ))),
            Form::Mongo(_) => None,
        };
        let route_value = match &form {
            Form::Ssh(f) => f.jump_host_id,
            Form::Redis(f) => f.tunnel_host_id,
            Form::Vnc(f) => f.vnc.tunnel_host_id,
            Form::Mongo(_) => None,
        };
        let route_select = route_choices.map(|choices| {
            let ix = index_of(&choices, &route_value);
            let state = cx.new(|cx| SelectState::new(choices, ix.map(IndexPath::new), window, cx));
            subscriptions.push(cx.subscribe_in(
                &state,
                window,
                |this, _, event: &SelectEvent<Vec<Choice<Option<i64>>>>, _, cx| {
                    let SelectEvent::Confirm(Some(id)) = event else {
                        return;
                    };
                    match &mut this.form {
                        Form::Ssh(f) => f.jump_host_id = *id,
                        Form::Redis(f) => f.tunnel_host_id = *id,
                        Form::Vnc(f) => f.vnc.tunnel_host_id = *id,
                        Form::Mongo(_) => {}
                    }
                    this.errors.remove(&Field::Uri);
                    cx.notify();
                },
            ));
            state
        });

        let vnc_route_select = match &form {
            Form::Ssh(f) => {
                let choices = to_choices(vnc_tunnel_choices(&tunnel_hosts(&hosts, self_id), true));
                let ix = index_of(&choices, &f.vnc.tunnel_host_id);
                let state =
                    cx.new(|cx| SelectState::new(choices, ix.map(IndexPath::new), window, cx));
                subscriptions.push(cx.subscribe_in(
                    &state,
                    window,
                    |this, _, event: &SelectEvent<Vec<Choice<Option<i64>>>>, _, cx| {
                        if let (SelectEvent::Confirm(Some(id)), Form::Ssh(f)) =
                            (event, &mut this.form)
                        {
                            f.vnc.tunnel_host_id = *id;
                            cx.notify();
                        }
                    },
                ));
                Some(state)
            }
            _ => None,
        };

        Self {
            kind,
            model,
            existing,
            form,
            inputs,
            group_select,
            route_select,
            vnc_route_select,
            hosts,
            errors: Errors::new(),
            has_secret,
            done: false,
            _subscriptions: subscriptions,
        }
    }

    fn editing(&self) -> bool {
        self.existing.is_some()
    }

    fn set_input(&self, which: In, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = self.inputs.get(&which) {
            let value = value.to_string();
            input.update(cx, |s, cx| s.set_value(value, window, cx));
        }
    }

    /// An input changed: update the form, and keep a URI and its fields in
    /// step in whichever direction the user typed.
    fn on_input(&mut self, which: In, value: String, window: &mut Window, cx: &mut Context<Self>) {
        // The field's error goes as soon as it is edited; the rest wait for
        // the next save.
        let field = match which {
            In::Name => Some(Field::Name),
            In::NewGroup => Some(Field::NewGroup),
            In::Host => Some(Field::Host),
            In::Port => Some(Field::Port),
            In::Username => Some(Field::Username),
            In::Password => Some(Field::Password),
            In::KeyPath => Some(Field::KeyPath),
            In::Uri => Some(Field::Uri),
            In::Target => Some(Field::Target),
            _ => None,
        };
        if let Some(f) = field {
            self.errors.remove(&f);
        }

        match which {
            In::Name => {
                self.form.common_mut().name = value;
                cx.notify();
                return;
            }
            In::NewGroup => {
                self.form.common_mut().new_group = value;
                cx.notify();
                return;
            }
            _ => {}
        }
        // Inputs to rewrite once the form is no longer borrowed.
        let mut push: Vec<(In, String)> = Vec::new();
        match &mut self.form {
            Form::Ssh(f) => match which {
                In::Host => f.host = value,
                In::Port => f.port = value,
                In::Username => f.username = value,
                In::Password => f.password = value,
                In::KeyPath => f.key_path = value,
                In::Target => f.vnc.target = value,
                In::VncPassword => f.vnc.password = value,
                In::VncUsername => f.vnc.username = value,
                _ => {}
            },
            Form::Vnc(f) => match which {
                In::Target => f.vnc.target = value,
                In::VncPassword => f.vnc.password = value,
                In::VncUsername => f.vnc.username = value,
                _ => {}
            },
            Form::Mongo(f) => match which {
                In::Uri => {
                    f.uri_typed(&value);
                    let x = &f.fields;
                    push.extend([
                        (In::Host, x.host.clone()),
                        (In::Port, x.port.clone()),
                        (In::Username, x.username.clone()),
                        (In::Password, x.password.clone()),
                        (In::Database, x.database.clone()),
                        (In::AuthSource, x.auth_source.clone()),
                        (In::Options, x.options.clone()),
                    ]);
                }
                In::Host
                | In::Port
                | In::Username
                | In::Password
                | In::Database
                | In::AuthSource
                | In::Options => {
                    let x = &mut f.fields;
                    match which {
                        In::Host => x.host = value,
                        In::Port => x.port = value,
                        In::Username => x.username = value,
                        In::Password => x.password = value,
                        In::Database => x.database = value,
                        In::AuthSource => x.auth_source = value,
                        _ => x.options = value,
                    }
                    f.fields_changed();
                    push.push((In::Uri, masked_uri(&f.uri)));
                }
                _ => {}
            },
            Form::Redis(f) => match which {
                In::Uri => {
                    f.uri_typed(&value);
                    let x = &f.fields;
                    push.extend([
                        (In::Host, x.host.clone()),
                        (In::Port, x.port.clone()),
                        (In::Database, x.database.clone()),
                        (In::Username, x.username.clone()),
                        (In::Password, x.password.clone()),
                    ]);
                }
                In::Host | In::Port | In::Database | In::Username | In::Password => {
                    let x = &mut f.fields;
                    match which {
                        In::Host => x.host = value,
                        In::Port => x.port = value,
                        In::Database => x.database = value,
                        In::Username => x.username = value,
                        _ => x.password = value,
                    }
                    f.fields_changed();
                    push.push((In::Uri, masked_uri(&f.uri)));
                }
                _ => {}
            },
        }
        for (which, value) in push {
            self.set_input(which, &value, window, cx);
        }
        cx.notify();
    }

    fn set_auth(&mut self, auth: AuthType, cx: &mut Context<Self>) {
        if let Form::Ssh(f) = &mut self.form {
            f.auth = auth;
            self.errors.remove(&Field::Password);
            self.errors.remove(&Field::KeyPath);
            cx.notify();
        }
    }

    fn browse_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        file_dialog::pick_file(window, cx, "Private key", None, move |path, window, cx| {
            let _ = this.update(cx, |this, cx| {
                let value = path.display().to_string();
                this.set_input(In::KeyPath, &value, window, cx);
                if let Form::Ssh(f) = &mut this.form {
                    f.key_path = value;
                }
                this.errors.remove(&Field::KeyPath);
                cx.notify();
            });
        });
    }

    /// Validate and write. Returns whether the dialog should close: on a
    /// validation error or a failed row write it stays open, as the Vue
    /// dialogs did. A secret that fails after the row was written is
    /// reported, and the dialog closes, because the row now exists and
    /// saving again would add it twice. A row whose stored URI still holds
    /// its password stores that password first and is left untouched, with
    /// the dialog open, when that fails (see [`run_save`]).
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.done {
            return false;
        }
        let plan = match self.form.plan(self.existing.as_ref()) {
            Ok(plan) => plan,
            Err(errors) => {
                self.errors = errors;
                cx.notify();
                return false;
            }
        };
        let id = self.existing.as_ref().map(|r| r.id);
        let model = self.model.clone();
        let outcome = run_save(
            &plan,
            id,
            cx,
            |cx, record| model.update(cx, |m, cx| m.save_host(id, record, cx)),
            |cx, kind, host_id, secret| model.read(cx).store_secret(kind, host_id, secret),
        );
        match outcome {
            SaveOutcome::Saved { failed, .. } => {
                for (kind, e) in failed {
                    toast(window, cx, secret_failed(kind, &e), ToastKind::Error);
                }
                self.done = true;
                true
            }
            SaveOutcome::RowFailed(e) => {
                toast(window, cx, self.kind.save_failed(&e), ToastKind::Error);
                false
            }
            SaveOutcome::SecretFailed(kind, e) => {
                toast(window, cx, secret_failed(kind, &e), ToastKind::Error);
                false
            }
        }
    }

    // ---- layout ----------------------------------------------------------

    fn input(&self, which: In) -> Input {
        let input = Input::new(&self.inputs[&which]);
        if matches!(which, In::Password | In::VncPassword) {
            input.mask_toggle()
        } else {
            input
        }
    }

    fn error(&self, field: Field, p: &Palette) -> Option<AnyElement> {
        self.errors.get(&field).map(|msg| {
            div()
                .text_xs()
                .text_color(p.bad)
                .child(msg.clone())
                .into_any_element()
        })
    }

    /// A labelled field: the label (with a red star when required), the
    /// control, then its error or else its hint.
    fn field(
        &self,
        label: impl Into<SharedString>,
        required: bool,
        control: impl IntoElement,
        error: Option<Field>,
        hint: Option<SharedString>,
        p: &Palette,
    ) -> AnyElement {
        let err = error.and_then(|f| self.error(f, p));
        v_flex()
            .gap_1()
            .min_w_0()
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(p.ink_2)
                    .child(label.into())
                    .when(required, |d| d.child(div().text_color(p.bad).child("*"))),
            )
            .child(control)
            .map(|d| match (err, hint) {
                (Some(e), _) => d.child(e),
                (None, Some(h)) => d.child(div().text_xs().text_color(p.ink_3).child(h)),
                (None, None) => d,
            })
            .into_any_element()
    }

    fn render_common(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let common = self.form.common();
        let new_group = common.group == GroupChoice::New;
        let favorite = common.favorite;
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(div().flex_1().min_w_0().child(self.field(
                        "Group",
                        false,
                        Select::new(&self.group_select).w_full(),
                        None,
                        None,
                        p,
                    )))
                    .when(new_group, |row| {
                        row.child(div().flex_1().min_w_0().child(self.field(
                            "New group name",
                            true,
                            self.input(In::NewGroup),
                            Some(Field::NewGroup),
                            None,
                            p,
                        )))
                    }),
            )
            .child(
                Checkbox::new("host-favorite")
                    .label("Favorite")
                    .checked(favorite)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.form.common_mut().favorite = *checked;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn render_ssh(&self, f: &SshForm, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let editing = self.editing();
        let auth_button = |id: &'static str, label: &'static str, auth: AuthType| {
            let selected = f.auth == auth;
            let b = Button::new(id).small().label(label).flex_1();
            let b = if selected { b.primary() } else { b.ghost() };
            b.on_click(cx.listener(move |this, _, _, cx| this.set_auth(auth, cx)))
        };
        let jump_hint: SharedString = "For a host only reachable through a bastion. The host and port above are resolved from the jump host, not from this machine.".into();
        let missing = jump_missing(f.jump_host_id, &self.hosts);

        let mut body = v_flex()
            .gap_3()
            .child(self.field(
                "Name",
                true,
                self.input(In::Name),
                Some(Field::Name),
                None,
                p,
            ))
            .child(self.field(
                "Host",
                true,
                self.input(In::Host),
                Some(Field::Host),
                None,
                p,
            ))
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(div().w(px(120.)).child(self.field(
                        "Port",
                        true,
                        self.input(In::Port),
                        Some(Field::Port),
                        None,
                        p,
                    )))
                    .child(div().flex_1().min_w_0().child(self.field(
                        "Username",
                        true,
                        self.input(In::Username),
                        Some(Field::Username),
                        None,
                        p,
                    ))),
            )
            .child(
                self.field(
                    "Authentication",
                    false,
                    h_flex()
                        .gap_1()
                        .p_1()
                        .rounded_sm()
                        .bg(p.input)
                        .child(auth_button("auth-password", "Password", AuthType::Password))
                        .child(auth_button("auth-key", "SSH Key", AuthType::Key)),
                    None,
                    None,
                    p,
                ),
            );
        body = match f.auth {
            AuthType::Password => body.child(self.field(
                "Password",
                !editing,
                self.input(In::Password),
                Some(Field::Password),
                editing.then(|| "Leave empty to keep the existing password".into()),
                p,
            )),
            AuthType::Key => body.child(
                self.field(
                    "Private Key Path",
                    true,
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().min_w_0().child(self.input(In::KeyPath)))
                        .child(
                            Button::new("browse-key")
                                .small()
                                .outline()
                                .icon(IconName::FileSearch)
                                .label("Browse")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.browse_key(window, cx)),
                                ),
                        ),
                    Some(Field::KeyPath),
                    Some("Supports ~ for home directory".into()),
                    p,
                ),
            ),
        };
        let route = self.route_select.as_ref().map(|s| {
            Select::new(s).w_full().placeholder(if missing {
                "A deleted host"
            } else {
                "Connect directly"
            })
        });
        body = body.child(
            v_flex()
                .gap_1()
                .child(div().text_xs().text_color(p.ink_2).child("Connect via"))
                .children(route)
                .child(if missing {
                    div().text_xs().text_color(p.bad).child(
                        "The jump host this host used has been deleted. Choose another, or connect directly.",
                    )
                } else {
                    div().text_xs().text_color(p.ink_3).child(jump_hint)
                }),
        );
        body = body.child(self.render_common(p, cx));
        body = body.child(self.render_ssh_vnc(f, p, cx));
        body.into_any_element()
    }

    /// An SSH host's remote desktop: off unless the row has one.
    fn render_ssh_vnc(&self, f: &SshForm, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let enabled = f.vnc_enabled;
        v_flex()
            .gap_3()
            .pt_3()
            .border_t_1()
            .border_color(p.line)
            .child(
                Checkbox::new("ssh-vnc")
                    .label("Remote desktop (VNC) on this host")
                    .checked(enabled)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        if let Form::Ssh(f) = &mut this.form {
                            f.vnc_enabled = *checked;
                            this.errors.remove(&Field::Target);
                            cx.notify();
                        }
                    })),
            )
            .when(enabled, |d| {
                d.child(self.render_vnc_fields(true, f.vnc.tunnel_host_id, f.vnc.view_only, p, cx))
            })
            .into_any_element()
    }

    /// `VncConnectionFields.vue`: address, password, user name, route and
    /// view-only. `is_ssh_host` changes the route's default and wording.
    fn render_vnc_fields(
        &self,
        is_ssh_host: bool,
        tunnel: Option<i64>,
        view_only: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let route = if is_ssh_host {
            self.vnc_route_select.as_ref()
        } else {
            self.route_select.as_ref()
        };
        let password_label: SharedString = if self.has_secret {
            "Password (stored)".into()
        } else {
            "Password".into()
        };
        v_flex()
            .gap_3()
            .child(self.field(
                "Address",
                true,
                self.input(In::Target),
                Some(Field::Target),
                Some(":1 means display 1 (port 5901). Use ::5901 for a literal port.".into()),
                p,
            ))
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(div().flex_1().min_w_0().child(self.field(
                        password_label,
                        false,
                        self.input(In::VncPassword),
                        None,
                        None,
                        p,
                    )))
                    .child(div().flex_1().min_w_0().child(self.field(
                        "User name (macOS)",
                        false,
                        self.input(In::VncUsername),
                        None,
                        None,
                        p,
                    ))),
            )
            .children(route.map(|s| {
                self.field(
                    "Connect through",
                    false,
                    Select::new(s).w_full(),
                    None,
                    Some(vnc_tunnel_hint(tunnel, is_ssh_host).into()),
                    p,
                )
            }))
            .child(
                Checkbox::new(if is_ssh_host {
                    "ssh-vnc-view-only"
                } else {
                    "vnc-view-only"
                })
                .label("View only — never send keyboard or mouse input")
                .checked(view_only)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    match &mut this.form {
                        Form::Ssh(f) => f.vnc.view_only = *checked,
                        Form::Vnc(f) => f.vnc.view_only = *checked,
                        _ => {}
                    }
                    cx.notify();
                })),
            )
            .into_any_element()
    }

    fn render_mongo(&self, f: &MongoForm, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let srv = f.is_srv();
        v_flex()
            .gap_3()
            .child(self.field(
                "Name",
                true,
                self.input(In::Name),
                Some(Field::Name),
                None,
                p,
            ))
            .child(
                v_flex()
                    .gap_3()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(p.line)
                    .child(section_title("Connection", p))
                    .child(self.field(
                        "Connection String",
                        true,
                        self.input(In::Uri),
                        Some(Field::Uri),
                        None,
                        p,
                    ))
                    .child(
                        h_flex()
                            .gap_3()
                            .items_start()
                            .child(div().flex_1().min_w_0().child(self.field(
                                "Host",
                                !srv,
                                self.input(In::Host),
                                Some(Field::Host),
                                None,
                                p,
                            )))
                            .child(div().w(px(120.)).child(self.field(
                                "Port",
                                !srv,
                                self.input(In::Port),
                                Some(Field::Port),
                                None,
                                p,
                            ))),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .items_start()
                            .child(div().flex_1().min_w_0().child(self.field(
                                "Username",
                                false,
                                self.input(In::Username),
                                None,
                                None,
                                p,
                            )))
                            .child(div().flex_1().min_w_0().child(self.field(
                                "Password",
                                false,
                                self.input(In::Password),
                                None,
                                self.has_secret.then(|| {
                                    "A password is stored. Leave blank to keep it.".into()
                                }),
                                p,
                            ))),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .items_start()
                            .child(div().flex_1().min_w_0().child(self.field(
                                "Database",
                                false,
                                self.input(In::Database),
                                None,
                                None,
                                p,
                            )))
                            .child(div().flex_1().min_w_0().child(self.field(
                                "Auth Source",
                                false,
                                self.input(In::AuthSource),
                                None,
                                None,
                                p,
                            ))),
                    )
                    .child(self.field(
                        "Connection Options",
                        false,
                        self.input(In::Options),
                        None,
                        None,
                        p,
                    )),
            )
            .child(self.render_common(p, cx))
            .into_any_element()
    }

    fn render_redis(&self, f: &RedisForm, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let tls = f.fields.tls;
        let tunnelled = f.tunnel_host_id.is_some();
        v_flex()
            .gap_3()
            .child(self.field("Name", false, self.input(In::Name), Some(Field::Name), None, p))
            .child(
                v_flex()
                    .gap_3()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(p.line)
                    .child(section_title("Connection", p))
                    .child(self.field(
                        "Connection URI",
                        false,
                        self.input(In::Uri),
                        Some(Field::Uri),
                        None,
                        p,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_start()
                            .child(div().flex_1().min_w_0().child(self.field(
                                "Host",
                                false,
                                self.input(In::Host),
                                None,
                                None,
                                p,
                            )))
                            .child(div().w(px(96.)).child(self.field(
                                "Port",
                                false,
                                self.input(In::Port),
                                None,
                                None,
                                p,
                            )))
                            .child(div().w(px(80.)).child(self.field(
                                "DB",
                                false,
                                self.input(In::Database),
                                None,
                                None,
                                p,
                            ))),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_start()
                            .child(div().flex_1().min_w_0().child(self.field(
                                "Username (ACL, Redis 6+)",
                                false,
                                self.input(In::Username),
                                None,
                                None,
                                p,
                            )))
                            .child(div().flex_1().min_w_0().child(self.field(
                                "Password",
                                false,
                                self.input(In::Password),
                                None,
                                None,
                                p,
                            ))),
                    )
                    .child(
                        Checkbox::new("redis-tls")
                            .label("Use TLS (rediss://)")
                            .checked(tls)
                            .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                let uri = match &mut this.form {
                                    Form::Redis(f) => {
                                        f.fields.tls = *checked;
                                        f.fields_changed();
                                        masked_uri(&f.uri)
                                    }
                                    _ => return,
                                };
                                this.errors.remove(&Field::Uri);
                                this.set_input(In::Uri, &uri, window, cx);
                                cx.notify();
                            })),
                    )
                    .children(self.route_select.as_ref().map(|s| {
                        self.field(
                            "Connect through SSH host",
                            false,
                            Select::new(s).w_full(),
                            None,
                            Some("For a Redis that only listens on a private network. The host and port above are resolved from the SSH host, not from this machine.".into()),
                            p,
                        )
                    }))
                    .when(tls && tunnelled, |d| {
                        d.child(div().text_xs().text_color(p.bad).child(
                            "A TLS connection cannot be tunnelled: the certificate names the real host, but through a tunnel only 127.0.0.1 is visible. The SSH tunnel already encrypts this hop — turn TLS off, or connect directly.",
                        ))
                    }),
            )
            .child(self.render_common(p, cx))
            .into_any_element()
    }

    fn render_vnc(&self, f: &VncForm, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_3()
            .child(self.field(
                "Name",
                true,
                self.input(In::Name),
                Some(Field::Name),
                None,
                p,
            ))
            .child(self.render_vnc_fields(false, f.vnc.tunnel_host_id, f.vnc.view_only, p, cx))
            .child(self.render_common(p, cx))
            .into_any_element()
    }
}

fn section_title(text: &'static str, p: &Palette) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.ink)
        .child(text)
}

impl Render for HostEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        // The form is read while rendering, then each part re-borrows `self`;
        // a clone keeps the two borrows apart and is a few strings.
        match &self.form {
            Form::Ssh(f) => {
                let f = f.clone();
                self.render_ssh(&f, &p, cx)
            }
            Form::Mongo(f) => {
                let f = f.clone();
                self.render_mongo(&f, &p, cx)
            }
            Form::Redis(f) => {
                let f = f.clone();
                self.render_redis(&f, &p, cx)
            }
            Form::Vnc(f) => {
                let f = f.clone();
                self.render_vnc(&f, &p, cx)
            }
        }
    }
}

/// Open the editor for `kind`: adding when `host_id` is `None` (into
/// `preset_group`, for "Add Host" on a group), editing otherwise.
pub fn open_host_editor(
    window: &mut Window,
    cx: &mut App,
    model: Entity<AppModel>,
    kind: EditorKind,
    host_id: Option<i64>,
    preset_group: Option<String>,
) {
    let existing = match host_id {
        None => None,
        Some(id) => match model.read(cx).host_record(id) {
            Ok(Some(rec)) => Some(rec),
            Ok(None) => {
                toast(window, cx, "This host no longer exists", ToastKind::Error);
                return;
            }
            Err(e) => {
                toast(
                    window,
                    cx,
                    format!("Could not load the host: {e}"),
                    ToastKind::Error,
                );
                return;
            }
        },
    };
    let editing = existing.is_some();
    let editor = cx.new(|cx| HostEditor::new(kind, model, existing, preset_group, window, cx));
    let first_build = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        // Focus the name once, on the first build only: a rebuild must not
        // pull focus back from wherever the user moved it.
        if first_build.replace(false) {
            let name = editor.read(cx).inputs.get(&In::Name).cloned();
            if let Some(name) = name {
                name.focus_handle(cx).focus(window, cx);
            }
        }
        let on_save = editor.clone();
        let on_enter = editor.clone();
        dialog
            .title(kind.title(editing))
            .w(px(500.))
            .child(editor.clone())
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("host-editor-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("host-editor-save")
                            .primary()
                            .label(kind.save_label(editing))
                            .on_click(move |_, window, cx| {
                                if on_save.update(cx, |e, cx| e.save(window, cx)) {
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
            // Enter in a field reaches the dialog as Confirm; save rather
            // than let the default close the dialog unsaved.
            .on_ok(move |_, window, cx| {
                if on_enter.update(cx, |e, cx| e.save(window, cx)) {
                    window.close_dialog(cx);
                }
                false
            })
    });
}
