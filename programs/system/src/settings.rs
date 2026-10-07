//! Settings: the themes, the living grain and the profile's face, the AI model, what gets
//! reported, and a reset.

use std::mem;

use icons::FACES;
use uiwire::{Event, Frame, Node, Request, Style, Variant};

use crate::{Disk, View, space, text};

/// The pages, one a line: Settings opens on the first.
const PAGES: &str = "Appearance\nAI\nPrivacy\nReset";
/// Widget ids: page `i` is `NAV + i`, theme `i` `THEME + i`, model `i` `MODEL + i`; the reports
/// switch, the feedback link and the grain's switch; the reset's word and its button; face `i`
/// `FACE + i`.
pub(crate) const NAV: u32 = 1;
pub(crate) const THEME: u32 = 10;
pub(crate) const MODEL: u32 = 20;
pub(crate) const REPORTS: u32 = 30;
pub(crate) const FEEDBACK: u32 = 31;
pub(crate) const LIVING: u32 = 32;
pub(crate) const WORD: u32 = 40;
pub(crate) const ERASE: u32 = 41;
pub(crate) const FACE: u32 = 50;
const FACE_NOTE: &str = "How the welcome shows you, above your name.";
/// The desktop's themes in its own order, which its theme cards' ids follow.
pub(crate) const THEMES: [&str; 4] = ["Dusk", "Dawn", "Mono Dark", "Mono Light"];
/// The models on offer as (name, then what it is best at; the value stored), the first the
/// default.
pub(crate) const MODELS: [(&str, &str); 2] =
    [("GLM 5.3\nbest answers", "zai/glm-5.3"), ("GLM 5.3 Flash\nfastest", "zai/glm-5.3-flash")];
const NOTE: &str = "AI is free while compusophy is in beta, powered by GLM 5.3. Your prompts go \
to the model through compusophy's server and are not stored there.";
const REPORT: &str = "Send error reports automatically";
const HOLDS: &str = "When something breaks (a program fails, the AI's server errs, or \
compusophyOS itself crashes) a short report goes to compusophy: what failed, the build, your \
browser and screen size, the theme, the apps open and the last 50 events, such as \u{201c}ai \
503\u{201d}. Never your files, your prompts or anything you typed.";
const TYPED: &str = "Feedback you write always sends: you choose what it says, and when.";
const FILES: &str = "Your files are kept only in this browser, on this device. The AI sees what \
you ask it to work on: an app you have Studio change, or the screen the Assistant reads.";
pub(crate) const UNKEPT: &str = "Your files could not be kept: this browser\u{2019}s storage is \
full or blocked, or another tab kept its own since. Changes made here are lost when the page \
reloads.";
const GRAIN: &str = "Living grain";
const GRAIN_NOTE: &str =
    "The backdrop's grain shifts, slightly. It stays still when your device asks for less motion.";
const RESET: &str = "Erase everything compusophy keeps in this browser: every profile, with its \
files, settings and PIN, and any reports waiting to send. compusophy then starts again as on a \
first visit, at the welcome. This cannot be undone.";
/// What the person types to reset, and what says so.
pub(crate) const SAY: &str = "reset";
const TYPE: &str = "Type reset to confirm.";
/// The widest row of cards.
const ROW_MAX: u16 = 440;

/// Settings: Appearance (the desktop's themes as cards, a click applying one; the living grain's
/// switch; then the faces, a click picking one, which the desktop keeps), AI (a note on the free
/// AI, the models as cards; a click picks one, which the desktop stores), Privacy (the automatic
/// reports switch, what a report holds, where files stay or that they could not be kept, a link
/// to Feedback) and Reset (what it erases; once `reset` is typed, Erase or Enter asks the
/// desktop to, [`Request::Reset`]), as the desktop's pages ([`Node::Pages`]). A profile's name
/// is the welcome's to change, and signing out the desktop's menu's.
#[derive(Debug)]
pub struct Settings {
    /// The page shown, the model the desktop answers with, and whether reports go, the grain
    /// lives and files are kept: as last set here or told by the desktop, and as it last told
    /// (it hears a change of ours only later: until its word changes, ours stands).
    pub(crate) page: u8,
    model: String,
    pub(crate) on: [bool; 3],
    told: Option<[bool; 3]>,
    /// The signed-in profile's face: as last picked here or told, and as told.
    pub(crate) face: u8,
    told_face: Option<u8>,
    /// What is typed to reset.
    pub(crate) word: String,
    /// The nodes last framed, the requests since, whether a frame went yet.
    shown: Vec<Node>,
    requests: Vec<Request>,
    framed: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        let (shown, requests, framed) = (Vec::new(), Vec::new(), false);
        Settings {
            page: 0,
            model: String::new(),
            on: [true; 3],
            told: None,
            face: 0,
            told_face: None,
            word: String::new(),
            shown,
            requests,
            framed,
        }
    }
}

impl View for Settings {
    fn event(&mut self, ev: &Event, _: &mut dyn Disk) -> bool {
        match *ev {
            Event::Config { ref model } => self.model.clone_from(model),
            Event::Prefs { reports, grain, kept } => {
                let now = [reports, grain, kept];
                let was = self.told.replace(now);
                for (i, on) in now.into_iter().enumerate() {
                    if was.is_none_or(|w| w[i] != on) {
                        self.on[i] = on;
                    }
                }
            }
            Event::Face { face } => {
                if self.told_face.replace(face) != Some(face) {
                    self.face = face;
                }
            }
            Event::Click { id } => self.click(id),
            Event::Change { id: WORD, ref text, .. } => self.word.clone_from(text),
            Event::Submit { id: WORD } => self.click(ERASE),
            _ => {}
        }
        let nodes = self.nodes();
        // Every Change gets a frame: the desktop sends the next one then.
        let edit = matches!(ev, Event::Change { .. });
        let changed = !self.framed || edit || !self.requests.is_empty() || nodes != self.shown;
        self.shown = nodes;
        changed
    }

    fn frame(&mut self) -> Frame {
        self.framed = true;
        let (requests, nodes) = (mem::take(&mut self.requests), self.shown.clone());
        Frame { seq: 0, title: "Settings".into(), requests, nodes }
    }
}

impl Settings {
    /// Whether the reset's word is typed.
    fn armed(&self) -> bool {
        self.word.trim().eq_ignore_ascii_case(SAY)
    }

    /// A click on widget `id`: a page, a face, a theme, a model, a switch, the feedback link or
    /// Erase.
    fn click(&mut self, id: u32) {
        let pref = |key: &str, value: &str| Request::Pref { key: key.into(), value: value.into() };
        if let Some(name) = THEMES.get(id.wrapping_sub(THEME) as usize) {
            self.requests.push(pref("theme", name));
        } else if let Some(&(_, model)) = MODELS.get(id.wrapping_sub(MODEL) as usize) {
            self.model = model.into();
            self.requests.push(pref("ai.model", model));
        } else if id == REPORTS || id == LIVING {
            let (key, i) = if id == REPORTS { ("reports", 0) } else { ("grain", 1) };
            self.on[i] = !self.on[i];
            self.requests.push(pref(key, if self.on[i] { "on" } else { "off" }));
        } else if id == FEEDBACK {
            self.requests.push(Request::Open { name: "feedback".into() });
        } else if id == ERASE && self.armed() {
            self.requests.push(Request::Reset);
        } else if id.wrapping_sub(FACE) < u32::from(FACES) {
            self.face = (id - FACE) as u8;
            self.requests.push(pref("face", &self.face.to_string()));
        } else if id.wrapping_sub(NAV) < 4 {
            self.page = (id - NAV) as u8;
        }
    }

    /// The pages, then the page shown.
    fn nodes(&self) -> Vec<Node> {
        let pages = Node::Pages { id: NAV, on: self.page, labels: PAGES.into() };
        let row = |child| Node::Pane { id: 0, w: ROW_MAX, children: vec![child] };
        let switch = |id, on, label: &str| row(Node::Switch { id, on, label: label.into() });
        let mut nodes = vec![pages];
        match self.page {
            0 => nodes.extend([
                text(Style::Heading, "Appearance"),
                text(Style::Small, "Pick a theme. The whole desktop follows at once."),
                space(0),
                Node::Themes { id: THEME },
                space(0),
                switch(LIVING, self.on[1], GRAIN),
                text(Style::Small, GRAIN_NOTE),
                space(0),
                text(Style::Subheading, "Your face"),
                text(Style::Small, FACE_NOTE),
                Node::Faces { id: FACE, on: self.face },
                // The view ends a spacing below the faces.
                space(0),
            ]),
            1 => {
                // A model not on offer (or none yet) is the default, as the desktop stores it.
                let on = MODELS.iter().position(|m| m.1 == self.model).unwrap_or(0);
                let models = MODELS
                    .iter()
                    .zip(MODEL..)
                    .enumerate()
                    .map(|(i, (m, id))| Node::Choice { id, on: i == on, text: m.0.into() });
                let note = vec![text(Style::Dim, NOTE), text(Style::Subheading, "Model")];
                nodes.extend([
                    text(Style::Heading, "AI"),
                    Node::Col { id: 0, gap: 12, children: note },
                    Node::Pane { id: 0, w: ROW_MAX, children: models.collect() },
                ]);
            }
            3 => {
                let (value, placeholder) = (self.word.clone(), SAY.into());
                let variant = if self.armed() { Variant::Danger } else { Variant::Normal };
                nodes.extend([
                    text(Style::Heading, "Reset"),
                    text(Style::Body, RESET),
                    space(0),
                    text(Style::Small, TYPE),
                    row(Node::Input { id: WORD, value, placeholder }),
                    row(Node::Button { id: ERASE, variant, label: "Erase everything".into() }),
                ]);
            }
            _ => nodes.extend([
                text(Style::Heading, "Privacy"),
                switch(REPORTS, self.on[0], REPORT),
                text(Style::Small, HOLDS),
                text(Style::Small, TYPED),
                if self.on[2] { text(Style::Small, FILES) } else { text(Style::Warning, UNKEPT) },
                space(0),
                row(Node::Button {
                    id: FEEDBACK,
                    variant: Variant::Link,
                    label: "Send feedback".into(),
                }),
            ]),
        }
        nodes
    }
}
