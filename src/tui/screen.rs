//! Navigation: the sidebar sections and every screen the TUI can show.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Sources,
    Destinations,
    Connections,
    Events,
    Dlq,
    Stats,
    Relay,
    Settings,
}

impl Section {
    pub const ALL: [Section; 8] = [
        Section::Sources,
        Section::Destinations,
        Section::Connections,
        Section::Events,
        Section::Dlq,
        Section::Stats,
        Section::Relay,
        Section::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Sources => "Sources",
            Section::Destinations => "Destinations",
            Section::Connections => "Connections",
            Section::Events => "Events",
            Section::Dlq => "DLQ",
            Section::Stats => "Stats",
            Section::Relay => "Relay",
            Section::Settings => "Settings",
        }
    }

    /// Labels for the one-line tab strip on narrow terminals.
    pub fn short_label(self) -> &'static str {
        match self {
            Section::Destinations => "Dests",
            Section::Connections => "Conns",
            other => other.label(),
        }
    }

    /// The name stored in `[ui.state] last_screen`.
    pub fn slug(self) -> &'static str {
        match self {
            Section::Sources => "sources",
            Section::Destinations => "destinations",
            Section::Connections => "connections",
            Section::Events => "events",
            Section::Dlq => "dlq",
            Section::Stats => "stats",
            Section::Relay => "relay",
            Section::Settings => "settings",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.slug() == slug)
    }

    /// The letter after `g`.
    pub fn jump_key(self) -> char {
        match self {
            Section::Sources => 's',
            Section::Destinations => 'd',
            Section::Connections => 'c',
            Section::Events => 'e',
            Section::Dlq => 'q',
            Section::Stats => 't',
            Section::Relay => 'r',
            Section::Settings => ',',
        }
    }

    pub fn from_jump_key(key: char) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|section| section.jump_key() == key)
    }

    pub fn root(self) -> Screen {
        match self {
            Section::Sources => Screen::Sources,
            Section::Destinations => Screen::Destinations,
            Section::Connections => Screen::Connections,
            Section::Events => Screen::Events,
            Section::Dlq => Screen::Dlq,
            Section::Stats => Screen::Stats,
            Section::Relay => Screen::Relay,
            Section::Settings => Screen::Settings,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceTab {
    Overview,
    Live,
    Events,
    Connections,
    Dlq,
}

impl SourceTab {
    pub const ALL: [SourceTab; 5] = [
        SourceTab::Overview,
        SourceTab::Live,
        SourceTab::Events,
        SourceTab::Connections,
        SourceTab::Dlq,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SourceTab::Overview => "Overview",
            SourceTab::Live => "Live",
            SourceTab::Events => "Events",
            SourceTab::Connections => "Connections",
            SourceTab::Dlq => "DLQ",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Login,
    Sources,
    SourceDetail { id: String, tab: SourceTab },
    Destinations,
    DestinationDetail { id: String },
    Connections,
    ConnectionDetail { id: String },
    EventDetail { id: String },
    Events,
    Dlq,
    Stats,
    Relay,
    Settings,
}

impl Screen {
    pub fn section(&self) -> Option<Section> {
        match self {
            Screen::Login => None,
            Screen::Sources | Screen::SourceDetail { .. } => Some(Section::Sources),
            Screen::Destinations | Screen::DestinationDetail { .. } => Some(Section::Destinations),
            Screen::Connections | Screen::ConnectionDetail { .. } => Some(Section::Connections),
            Screen::EventDetail { .. } => Some(Section::Events),
            Screen::Events => Some(Section::Events),
            Screen::Dlq => Some(Section::Dlq),
            Screen::Stats => Some(Section::Stats),
            Screen::Relay => Some(Section::Relay),
            Screen::Settings => Some(Section::Settings),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jump_keys_match_the_spec_and_are_unique() {
        let keys: Vec<char> = Section::ALL
            .iter()
            .map(|section| section.jump_key())
            .collect();
        assert_eq!(keys, vec!['s', 'd', 'c', 'e', 'q', 't', 'r', ',']);
        for section in Section::ALL {
            assert_eq!(Section::from_jump_key(section.jump_key()), Some(section));
            assert_eq!(Section::from_slug(section.slug()), Some(section));
        }
        assert_eq!(Section::from_jump_key('x'), None);
    }

    #[test]
    fn screens_belong_to_their_section() {
        let detail = Screen::SourceDetail {
            id: "s".into(),
            tab: SourceTab::Live,
        };
        assert_eq!(detail.section(), Some(Section::Sources));
        assert_eq!(
            Screen::ConnectionDetail { id: "c".into() }.section(),
            Some(Section::Connections)
        );
        assert_eq!(Screen::Login.section(), None);
        assert_eq!(Section::Dlq.root(), Screen::Dlq);
    }

    #[test]
    fn source_tabs_wrap() {
        assert_eq!(SourceTab::Dlq.next(), SourceTab::Overview);
        assert_eq!(SourceTab::Overview.previous(), SourceTab::Dlq);
        assert_eq!(SourceTab::ALL[3], SourceTab::Connections);
    }
}
