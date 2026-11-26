//! Guide chapters the controller reads instead of inventing a manual.

pub struct Chapter {
    pub id: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

pub fn chapters() -> &'static [Chapter] {
    &CHAPTERS
}

pub fn get(id: &str) -> Option<&'static Chapter> {
    CHAPTERS.iter().find(|chapter| chapter.id == id)
}

const CHAPTERS: [Chapter; 8] = [
    Chapter {
        id: "start",
        title: "Starting a session",
        body: include_str!("start.md"),
    },
    Chapter {
        id: "being",
        title: "The being",
        body: include_str!("being.md"),
    },
    Chapter {
        id: "memory",
        title: "Memory",
        body: include_str!("memory.md"),
    },
    Chapter {
        id: "decide",
        title: "Decisions",
        body: include_str!("decide.md"),
    },
    Chapter {
        id: "tools",
        title: "Sandbox tools",
        body: include_str!("tools.md"),
    },
    Chapter {
        id: "change",
        title: "Change sets and promotion",
        body: include_str!("change.md"),
    },
    Chapter {
        id: "loop",
        title: "The autonomous loop",
        body: include_str!("loop.md"),
    },
    Chapter {
        id: "chat",
        title: "Talking to the controller",
        body: include_str!("chat.md"),
    },
];

/// Top chapters by token overlap. A token is a lowercase word of three or more letters.
pub fn search(query: &str, limit: usize) -> Vec<Chunk<'_>> {
    let needles = tokens(query);
    let mut scored: Vec<(usize, Chunk)> = Vec::new();
    for chapter in &CHAPTERS {
        for (index, chunk) in chapter.body.split("\n\n").enumerate() {
            let hay = tokens(chunk);
            let score = needles.iter().filter(|needle| hay.contains(needle)).count();
            if score == 0 {
                continue;
            }
            scored.push((
                score,
                Chunk {
                    id: chapter.id,
                    title: chapter.title,
                    excerpt: chunk.trim(),
                    index,
                },
            ));
        }
    }
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.id.cmp(b.1.id))
            .then(a.1.index.cmp(&b.1.index))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, chunk)| chunk)
        .collect()
}

pub struct Chunk<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub excerpt: &'a str,
    pub index: usize,
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}
