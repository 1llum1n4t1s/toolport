//! Lossless query evidence alongside the existing positive retrieval projection.
//! Polarity describes explicit exclusion markers, not inferred compatibility.
use serde::Serialize;
use std::ops::Range;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Positive,
    Excluded,
}

#[derive(Debug, Serialize)]
pub struct Span<'a> {
    pub bytes: Range<usize>,
    pub text: &'a str,
    pub polarity: Polarity,
    pub marker: Option<&'a str>,
}

#[derive(Debug, Serialize)]
pub struct Query<'a> {
    pub original: &'a str,
    pub positive: String,
    pub spans: Vec<Span<'a>>,
}

impl<'a> Query<'a> {
    pub fn parse(original: &'a str) -> Self {
        let mut spans = Vec::new();
        let mut sentence_start = 0;
        for sentence in original.split([',', ';', '.', '!', '?']) {
            // ASCII folding keeps byte offsets valid for Unicode and camelCase.
            let lower = sentence.to_ascii_lowercase();
            let mut separators: Vec<_> = lower
                .match_indices(" and ")
                .chain(lower.match_indices(" but "))
                .collect();
            separators.sort_by_key(|(position, _)| *position);
            let mut start = 0;
            for (end, separator) in separators
                .into_iter()
                .chain(std::iter::once((sentence.len(), "")))
            {
                if end < start {
                    continue;
                }
                let folded = &lower[start..end];
                let excluded = ["do not ", "don't ", "dont ", "without ", "not "]
                    .iter()
                    .filter_map(|marker| {
                        folded
                            .find(marker)
                            .filter(|position| {
                                *position == 0
                                    || folded.as_bytes()[position - 1].is_ascii_whitespace()
                            })
                            .map(|position| (position, marker.len()))
                    })
                    .min_by_key(|(position, _)| *position);
                let positive_end = start + excluded.map_or(end - start, |(position, _)| position);
                let bytes = sentence_start + start..sentence_start + positive_end;
                spans.push(Span {
                    text: &original[bytes.clone()],
                    bytes,
                    polarity: Polarity::Positive,
                    marker: None,
                });
                if let Some((_, marker_length)) = excluded {
                    let bytes = sentence_start + positive_end..sentence_start + end;
                    spans.push(Span {
                        marker: Some(&original[bytes.start..bytes.start + marker_length]),
                        text: &original[bytes.clone()],
                        bytes,
                        polarity: Polarity::Excluded,
                    });
                }
                start = end + separator.len();
            }
            sentence_start += sentence.len() + 1;
        }
        let positive = spans
            .iter()
            .filter(|span| span.polarity == Polarity::Positive)
            .map(|span| span.text)
            .collect::<Vec<_>>()
            .join(" ");
        Self {
            original,
            positive,
            spans,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_distinct_constraints_and_positive_modifiers() {
        let a = Query::parse(
            "İ getItem quietly without deletingAnything; readTextFile but do not overwrite bytes",
        );
        let b = Query::parse(
            "İ getItem quietly without creatingAnything; readTextFile but do not overwrite bytes",
        );
        assert_eq!(a.positive, b.positive);
        assert!(a.positive.contains("quietly"));
        assert!(a.positive.contains("readTextFile"));
        assert!(!a.positive.contains("deletingAnything"));
        let excluded: Vec<_> = a
            .spans
            .iter()
            .filter(|s| s.polarity == Polarity::Excluded)
            .collect();
        assert_eq!(excluded.len(), 2);
        assert_eq!(excluded[0].text, "without deletingAnything");
        assert_eq!(excluded[1].text, "do not overwrite bytes");
        assert_ne!(a.original, b.original);
        assert_ne!(a.spans[1].text, b.spans[1].text);
        for span in &a.spans {
            assert_eq!(&a.original[span.bytes.clone()], span.text);
        }
        assert_eq!(excluded[0].marker, Some("without "));
    }

    #[test]
    fn keeps_full_punctuation_and_does_not_infer_negation() {
        let original =
            "read only metadata?! notebook quietly, don't delete; not update AND getItem";
        let parsed = Query::parse(original);
        assert_eq!(parsed.original, original);
        assert!(parsed.positive.contains("read only metadata"));
        assert!(parsed.positive.contains("notebook quietly"));
        assert!(parsed.positive.contains("getItem"));
        assert_eq!(
            parsed
                .spans
                .iter()
                .filter(|s| s.polarity == Polarity::Excluded)
                .count(),
            2
        );
    }
}
