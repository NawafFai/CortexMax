//! Who a sentence spoken in a meeting is for.
//!
//! Every transcribed sentence used to mention every agent in the meeting, and
//! an agent answers whenever it is mentioned — so seven agents answered every
//! sentence at once, talking over each other and over the owner. A sentence
//! now reaches only the agents it names ("Pilot, …", "Axel and Judge …"); a
//! call to the whole room ("everyone", "team") reaches them all; and a sentence
//! that names nobody goes to the lead alone, who can bring others in.

use std::collections::HashMap;

/// The agent a sentence naming nobody is for.
const LEAD_AGENT: &str = "cortex";

/// Words that call the whole room, in English and Arabic.
const EVERYONE: &[&str] = &[
    "everyone",
    "everybody",
    "team",
    "all",
    "guys",
    "الكل",
    "الجميع",
    "الفريق",
    "ياجماعه",
    "جماعه",
    "كلكم",
];

/// How the agents' names come out of Arabic speech recognition.
const ARABIC_NAMES: &[(&str, &[&str])] = &[
    ("cortex", &["كورتكس", "كورتيكس"]),
    ("pilot", &["بايلوت", "بيلوت"]),
    ("axel", &["اكسل", "آكسل"]),
    ("judge", &["جدج", "جادج"]),
    ("syntax", &["سنتاكس", "سينتاكس"]),
    ("probe", &["بروب"]),
    ("prism", &["بريزم", "برزم"]),
    ("aegis", &["ايجيس", "ايجس"]),
    ("orion", &["اوريون"]),
    ("scope", &["سكوب"]),
    ("kernel", &["كيرنل", "كرنل"]),
    ("nexus", &["نكسس", "نيكسس"]),
    ("relay", &["ريلاي"]),
    ("vector", &["فيكتور", "فكتور"]),
    ("vault", &["فولت"]),
    ("pixel", &["بكسل", "بيكسل"]),
    ("signal", &["سيقنال", "سيغنال", "سجنال"]),
    ("comply", &["كومبلاي"]),
];

/// Arabic letters folded so spelling variants compare equal.
fn fold(word: &str) -> String {
    word.chars()
        .filter(|c| !('\u{064B}'..='\u{0652}').contains(c) && *c != '\u{0640}')
        .map(|c| match c {
            'أ' | 'إ' | 'آ' | 'ء' | 'ؤ' | 'ئ' => 'ا',
            'ة' => 'ه',
            'ى' => 'ي',
            _ => c,
        })
        .collect::<String>()
        .to_lowercase()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// Whether the spoken words name `name` — exactly in English, or within one
/// letter of an Arabic spelling (speech recognition splits and bends names:
/// "كورا تكس", "بأولوت").
fn speaks_of(words: &[String], name: &str) -> bool {
    if words.iter().any(|word| word == name) {
        return true;
    }
    let Some((_, spellings)) = ARABIC_NAMES.iter().find(|(english, _)| *english == name) else {
        return false;
    };
    let mut candidates: Vec<String> = words.iter().map(|w| fold(w)).collect();
    candidates.extend(
        words
            .windows(2)
            .map(|pair| fold(&format!("{}{}", pair[0], pair[1]))),
    );
    candidates.iter().any(|word| {
        let word = word
            .strip_prefix("يا")
            .filter(|rest| rest.chars().count() >= 3)
            .unwrap_or(word);
        spellings.iter().any(|spelling| {
            edit_distance(word, &fold(spelling)) <= usize::from(spelling.chars().count() >= 4)
        })
    })
}

/// The agents (pubkeys, as given) this sentence is for. `names` maps a
/// lower-case pubkey to the agent's first name.
pub(crate) fn addressed_agents(
    text: &str,
    agents: &[String],
    names: &HashMap<String, String>,
) -> Vec<String> {
    if agents.is_empty() {
        return Vec::new();
    }
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let name_of = |pubkey: &String| {
        names
            .get(&pubkey.to_ascii_lowercase())
            .map(|name| name.to_lowercase())
    };
    let named: Vec<String> = agents
        .iter()
        .filter(|pubkey| name_of(pubkey).is_some_and(|name| speaks_of(&words, &name)))
        .cloned()
        .collect();
    // A name wins over a group word: "Pilot, split the tasks across the team"
    // is for Pilot.
    if !named.is_empty() {
        return named;
    }
    if words
        .iter()
        .any(|word| EVERYONE.contains(&fold(word).as_str()))
    {
        return agents.to_vec();
    }
    let lead = agents
        .iter()
        .find(|pubkey| name_of(pubkey).is_some_and(|name| name.starts_with(LEAD_AGENT)))
        .unwrap_or(&agents[0]);
    vec![lead.clone()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> (Vec<String>, HashMap<String, String>) {
        let agents = vec!["aa".to_string(), "bb".to_string(), "cc".to_string()];
        let names = HashMap::from([
            ("aa".to_string(), "Pilot".to_string()),
            ("bb".to_string(), "CORTEX".to_string()),
            ("cc".to_string(), "Axel".to_string()),
        ]);
        (agents, names)
    }

    #[test]
    fn a_named_agent_alone_hears_the_sentence() {
        let (agents, names) = room();
        assert_eq!(
            addressed_agents("Axel, what is the status?", &agents, &names),
            ["cc"]
        );
        assert_eq!(
            addressed_agents("pilot and axel check the tests", &agents, &names),
            ["aa", "cc"]
        );
    }

    #[test]
    fn a_sentence_naming_nobody_goes_to_the_lead() {
        let (agents, names) = room();
        assert_eq!(
            addressed_agents("what did we finish today", &agents, &names),
            ["bb"]
        );
    }

    #[test]
    fn a_call_to_the_room_reaches_everyone() {
        let (agents, names) = room();
        assert_eq!(
            addressed_agents("Okay team, listen up", &agents, &names),
            agents
        );
    }

    #[test]
    fn without_the_lead_the_first_agent_answers_and_an_empty_room_nobody() {
        let (_, names) = room();
        let agents = vec!["cc".to_string(), "aa".to_string()];
        assert_eq!(addressed_agents("hello", &agents, &names), ["cc"]);
        assert!(addressed_agents("hello", &[], &names).is_empty());
    }

    #[test]
    fn arabic_speech_names_the_agent_even_when_bent() {
        let (agents, names) = room();
        assert_eq!(
            addressed_agents("بأولوت وزع المهام على الفريق", &agents, &names),
            ["aa"]
        );
        assert_eq!(
            addressed_agents("يا اكسل وش الوضع", &agents, &names),
            ["cc"]
        );
        assert_eq!(
            addressed_agents("كورا تكس، واش آخر تحديث", &agents, &names),
            ["bb"]
        );
        assert_eq!(addressed_agents("الكل يسمعني", &agents, &names), agents);
        assert_eq!(
            addressed_agents("وش صار في المشروع", &agents, &names),
            ["bb"]
        );
    }

    #[test]
    fn names_match_whole_words_only() {
        let (agents, names) = room();
        assert_eq!(
            addressed_agents("the autopilot failed", &agents, &names),
            ["bb"]
        );
    }
}
