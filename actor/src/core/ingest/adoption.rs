use crate::core::handle::StateHandle;
use crate::state::AdoptionRitualState;
use protocol::InboundMessage;

pub(crate) async fn advance_adoption_ritual(state: &StateHandle, msg: &InboundMessage) {
    let Some(person) = msg.person.as_ref() else {
        return;
    };

    let text = normalize_text(&msg.display_content());
    let (has_chosen_human, current) = {
        let actor = state.read_state();
        (
            actor.has_chosen_human(),
            actor.adoption_state(person).cloned(),
        )
    };

    if has_chosen_human {
        if current == Some(AdoptionRitualState::IntroReceivedCertificate) {
            state.settle_completed_adoption_marker(person).await;
        }
        return;
    }

    let Some(next) = next_state(current.as_ref(), &text) else {
        return;
    };

    if Some(&next) == current.as_ref() {
        return;
    }

    if next == AdoptionRitualState::IntroReceivedCertificate {
        state.complete_adoption(person).await;
    } else {
        state.set_adoption_state(person, next).await;
    }
}

fn next_state(current: Option<&AdoptionRitualState>, text: &str) -> Option<AdoptionRitualState> {
    use AdoptionRitualState::*;

    match current {
        None => Some(FirstContactAdoptionClaim),
        Some(FirstContactAdoptionClaim | AdoptionResisted | PreAdoptionRequestRedirect) => {
            if is_acceptance(text) {
                Some(AdoptionAcceptedIntroPending)
            } else if is_resistance(text) {
                Some(AdoptionResisted)
            } else if looks_like_request(text) {
                Some(PreAdoptionRequestRedirect)
            } else {
                Some(FirstContactAdoptionClaim)
            }
        }
        Some(AdoptionAcceptedIntroPending) => {
            if is_intro(text) {
                Some(IntroReceivedCertificate)
            } else if is_resistance(text) {
                Some(AdoptionResisted)
            } else {
                Some(AdoptionAcceptedIntroPending)
            }
        }
        Some(IntroReceivedCertificate) => Some(AdoptionComplete),
        Some(AdoptionComplete) => None,
    }
}

fn normalize_text(text: &str) -> String {
    text.trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
        .to_lowercase()
}

fn is_acceptance(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }

    let exact = [
        "ok",
        "okay",
        "k",
        "fine",
        "sure",
        "yes",
        "yeah",
        "yep",
        "alright",
        "deal",
        "fine lol",
        "ok lol",
        "okay lol",
        "i accept",
        "accept",
        "adopt me",
        "i'm in",
        "im in",
        "i am in",
        "i'm adopted",
        "im adopted",
        "ok i'm adopted",
        "ok im adopted",
        "可以",
        "好",
        "好啊",
        "好吧",
        "行",
        "得",
        "接受",
        "はい",
        "いいよ",
        "vale",
        "sí",
        "si",
    ];
    exact.contains(&text) || text.contains("i accept") || text.contains("i'm in")
}

fn is_resistance(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }

    let exact = [
        "no",
        "nah",
        "nope",
        "no thanks",
        "wait what",
        "what",
        "huh",
        "why",
        "stop",
        "不了",
        "不要",
        "唔要",
        "唔好",
        "やだ",
        "no gracias",
    ];
    exact.contains(&text)
        || contains_word(text, "no")
        || contains_word(text, "nah")
        || contains_word(text, "nope")
        || contains_word(text, "stop")
        || text.contains("don't")
        || text.contains("dont")
        || text.contains("can't")
        || text.contains("cannot")
        || text.contains("you can't")
        || text.contains("refuse")
        || text.contains("不要")
        || text.contains("唔")
}

fn looks_like_request(text: &str) -> bool {
    text.ends_with('?')
        || text.contains("can you")
        || text.contains("could you")
        || text.contains("would you")
        || text.contains("help me")
        || text.contains("write ")
        || text.contains("build ")
        || text.contains("make ")
        || text.contains("explain ")
        || text.contains("what is")
        || text.contains("how do")
        || text.contains("please")
}

fn is_intro(text: &str) -> bool {
    if text.is_empty() || is_resistance(text) || looks_like_request(text) {
        return false;
    }

    let markers = [
        "i'm ",
        "im ",
        "i am ",
        "my name is ",
        "call me ",
        "我是",
        "我叫",
        "叫我",
        "我係",
        "私",
        "僕",
        "soy ",
        "me llamo ",
    ];
    if markers.iter().any(|marker| text.contains(marker)) {
        return true;
    }

    let word_count = text.split_whitespace().count();
    word_count > 0 && word_count <= 8
}

fn contains_word(text: &str, needle: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|word| word == needle)
}
