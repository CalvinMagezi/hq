//! What an `.xlsx` workbook holds that its Markdown tables cannot: formulas, charts, pictures,
//! merged ranges, conditional formats and validation rules. The reader shows values only, so
//! each of these is counted and reported as a loss rather than dropped silently.
//!
//! A tag scan is enough: only counts are wanted, and a sheet can hold millions of cells.

use super::package::Package;
use super::report::{LossKind, Losses};

/// A sheet part bigger than this is not scanned; a formula count would not be worth the time.
const MAX_SHEET_BYTES: usize = 24 * 1024 * 1024;

pub fn scan(pkg: &mut Package<'_>, losses: &mut Losses) {
    let names: Vec<String> = pkg.names().to_vec();
    let mut formulas = 0;
    for name in &names {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("xl/charts/chart") && lower.ends_with(".xml") {
            losses.hit(LossKind::ChartsOmitted);
        } else if lower.starts_with("xl/media/") {
            losses.hit(LossKind::EmbeddedObjectsOmitted);
        } else if lower.starts_with("xl/worksheets/") && lower.ends_with(".xml") {
            let Ok(Some(bytes)) = pkg.read(name) else {
                continue;
            };
            if bytes.len() > MAX_SHEET_BYTES {
                continue;
            }
            let xml = super::package::text_of(bytes);
            formulas += count_tags(&xml, "f");
            losses.hit_n(
                LossKind::MergedRangesUnmerged,
                count_tags(&xml, "mergeCell"),
            );
            losses.hit_n(
                LossKind::ConditionalFormatsDropped,
                count_tags(&xml, "conditionalFormatting"),
            );
            losses.hit_n(
                LossKind::ValidationRulesDropped,
                count_tags(&xml, "dataValidation"),
            );
        }
    }
    losses.hit_n(LossKind::FormulasShownAsValues, formulas);
}

/// How many `<name ...>` or `<ns:name ...>` start tags the XML holds. A longer name that merely
/// begins with `name` (`<fooBar` for `foo`) does not count.
fn count_tags(xml: &str, name: &str) -> usize {
    let mut n = 0;
    let mut rest = xml;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let tag_end = rest
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(rest.len());
        let tag = &rest[..tag_end];
        if tag == name || tag.rsplit_once(':').is_some_and(|(_, local)| local == name) {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::count_tags;

    #[test]
    fn tags_are_counted_by_exact_name_with_or_without_a_prefix() {
        let xml = r#"<c><f>A1+1</f></c><c><f t="shared" si="0"/></c><x:f>2</x:f><fooBar/><dataValidations count="1"><dataValidation/></dataValidations>"#;
        assert_eq!(count_tags(xml, "f"), 3);
        assert_eq!(count_tags(xml, "dataValidation"), 1);
        assert_eq!(count_tags(xml, "foo"), 0);
    }
}
