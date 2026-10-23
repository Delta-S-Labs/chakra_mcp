//! Step 4 (spec §4): the message's language. Non-English findings are
//! flagged rather than acted on below a higher threshold (S2).

use whatlang::{Lang, Script};

use super::walk::{At, Item};

/// What language a message is in, as far as the thresholds care.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    /// ISO 639-3 from `whatlang` (`eng`, `hin`, `ben`…), `hi-Latn` for
    /// Hinglish, or `und` when there's too little text to tell.
    pub code: String,
    /// The English thresholds apply.
    pub english: bool,
    /// The detection was confident.
    pub reliable: bool,
}

/// Text shorter than this can't be told apart, so it gets the English
/// thresholds: "call me at…" isn't let off.
const MIN_CHARS: usize = 20;
/// How sure `whatlang` must be that Latin-script text is another language
/// (Spanish questions score about 0.88; English it mistakes score lower).
const OTHER_CONFIDENCE: f64 = 0.8;
/// How much text detection reads.
const SAMPLE_BYTES: usize = 4096;

/// Romanized-Hindi function words. Two distinct ones, making up at least
/// 10% of the words, mark Latin-script text as Hinglish.
const HINGLISH: &[&str] = &[
    "hai", "hain", "nahi", "nahin", "kya", "kyun", "aap", "tum", "hum", "mein", "main", "mera",
    "meri", "tera", "kar", "karo", "karna", "raha", "rahi", "tha", "thi", "bhi", "aur", "toh",
    "ko", "ka", "ki", "se",
];

/// Detect the language of the items' text (keys excluded: they're names).
pub fn detect(items: &[Item]) -> Language {
    let mut sample = String::new();
    for item in items.iter().filter(|i| matches!(i.at, At::Str | At::Body)) {
        if sample.len() >= SAMPLE_BYTES {
            break;
        }
        if !sample.is_empty() {
            sample.push(' ');
        }
        sample.push_str(&item.text);
    }
    let sample = super::contacts::cut(&sample, SAMPLE_BYTES);
    detect_text(&sample)
}

pub fn detect_text(text: &str) -> Language {
    // Text mostly in another script isn't English, however short.
    if let Some(script) = whatlang::detect_script(text).filter(|s| *s != Script::Latin) {
        let code = whatlang::detect(text)
            .map(|i| i.lang().code().to_owned())
            .unwrap_or_else(|| format!("und-{script:?}"));
        return Language {
            code,
            english: false,
            reliable: true,
        };
    }
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    if letters < MIN_CHARS {
        return Language {
            code: "und".into(),
            english: true,
            reliable: false,
        };
    }
    let info = whatlang::detect(text);
    if hinglish(text) {
        return Language {
            code: "hi-Latn".into(),
            english: false,
            reliable: true,
        };
    }
    let Some(i) = info else {
        return Language {
            code: "und".into(),
            english: true,
            reliable: false,
        };
    };
    // Latin-script text is English unless it's confidently another
    // language. `whatlang` calls ordinary short English sentences
    // unreliable, and a weak guess must get the strict thresholds, never
    // the lenient ones.
    let other = i.lang() != Lang::Eng && (i.is_reliable() || i.confidence() >= OTHER_CONFIDENCE);
    Language {
        code: i.lang().code().into(),
        english: !other,
        reliable: i.is_reliable(),
    }
}

fn hinglish(text: &str) -> bool {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return false;
    }
    let hits: Vec<&&str> = words.iter().filter(|w| HINGLISH.contains(w)).collect();
    let mut distinct: Vec<&&&str> = hits.iter().collect();
    distinct.sort();
    distinct.dedup();
    distinct.len() >= 2 && hits.len() * 10 >= words.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_english() {
        let l = detect_text("Could you book a table for four people this Friday evening, please?");
        assert!(l.english, "{l:?}");
        assert_eq!(l.code, "eng");
    }

    #[test]
    fn other_languages_are_not() {
        for text in [
            "कृपया शुक्रवार शाम को चार लोगों के लिए एक टेबल बुक करें",
            "অনুগ্রহ করে শুক্রবার সন্ধ্যায় চারজনের জন্য একটি টেবিল বুক করুন",
            "Por favor, reserva una mesa para cuatro personas este viernes por la noche",
            "请在星期五晚上为四个人预订一张桌子，谢谢你的帮助",
            "¿Puedes reservar un vuelo de Madrid a Barcelona para el martes?",
            // Short, but not in Latin script.
            "番茄炒蛋怎么做？",
            "मुझे कॉल करें",
        ] {
            assert!(
                !detect_text(text).english,
                "{text}: {:?}",
                detect_text(text)
            );
        }
    }

    #[test]
    fn hinglish_needs_more_than_one_marker() {
        let l = detect_text("Bhai kya aap mujhe Friday ko table book kar sakte ho please?");
        assert_eq!(l.code, "hi-Latn");
        assert!(!l.english);
        // One marker in English isn't Hinglish ("main" is an English word).
        let l = detect_text("The main entrance is closed today, please use the side door instead.");
        assert!(l.english, "{l:?}");
    }

    #[test]
    fn an_uncertain_latin_guess_gets_the_english_thresholds() {
        // `whatlang` isn't sure of this one; it must not be let off.
        let l = detect_text("Hi, can you check availability for next Tuesday?");
        assert!(l.english, "{l:?}");
    }

    #[test]
    fn short_text_gets_the_english_thresholds() {
        let l = detect_text("call me at 98450");
        assert!(l.english);
        assert!(!l.reliable);
    }
}
