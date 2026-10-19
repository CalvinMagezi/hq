#!/usr/bin/env bash
# Fetch real public test documents for the import pipeline.
#
# The files come from Apache Tika's test corpus (Apache License 2.0). They are
# downloaded on demand into target/conversion-corpus and never committed.
#
#   ./scripts/fetch-conversion-corpus.sh
#   HQ_CONVERT_CORPUS=target/conversion-corpus \
#     cargo test -p hq-convert --test real_corpus -- --ignored --nocapture
#
# Needs git and network access to github.com.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="${1:-$root/target/conversion-corpus}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

picks=(
  testWORD.docx testWORD_boldHyperlink.docx testWORD_bold_character_runs.docx testWORD_features.docx
  testWORD_endnote_table.docx testWORD_tableInRunInsideParagraph.docx testWORD_1img.docx testWORD_characterStyles.docx
  testWORD.doc testWORD6.doc testWORD_boldHyperlink.doc testWORD_features.doc testWORD_bold_character_runs.doc
  testEXCEL.xlsx testEXCEL.xls testEXCEL_hyperlinks.xlsx testEXCEL_dateFormats.xlsx testEXCEL_big_numbers.xlsx
  testEXCEL-formats.xlsx testEXCEL_labels-govdocs-515858.xls testEXCEL_headers_footers.xlsx
  testPPT.pptx testPPT_various.pptx testPPT_masterText.pptx testPPT_groups.pptx testPPT_2imgs.pptx
  testPPT.ppt testPPT_various.ppt
  testRTF.rtf testRTFBoldItalic.rtf testRTFHyperlink.rtf testRTFListMicrosoftWord.rtf testRTFListLibreOffice.rtf
  testRTFTableCellSeparation.rtf testRTFUmlautSpaces.rtf testRTFVarious.rtf testRTFJapanese.rtf
  testRTFWithCurlyBraces.rtf testRTF_annotation_spacing.rtf
  testOpenOffice2.odt testODFwithOOo3.odt testODTStyles2.odt testStyles.odt testFooter.odt testODTEmbedded.odt
  test-columnar.ods testFooter.ods testMasterFooter.odp testODTnotaZipFile.odt testODTEncrypted.odt
  testXML.xml testXML2.xml testTEI.xml testONIXMessageShort.xml example.xml testXXE.xml undeclared_entity.xml
  testHTML.html testHTML_utf8.html testHTML_charset_utf16le.html testHTML_embedded_img.html testHTMLNoisyMetaEncoding_1.html
  testPDF.pdf testPDFVarious.pdf testOCR.pdf testPDFTwoTextBoxes.pdf testPDF_Version.8.x.pdf
  testOCR.png testOCR.jpg testOCR_spacing.png
)

git clone --quiet --depth 1 --filter=blob:none --no-checkout https://github.com/apache/tika.git "$work/tika"
cd "$work/tika"
git ls-tree -r HEAD --name-only | grep '/test-documents/' > "$work/paths.txt"

: > "$work/selected.txt"
for name in "${picks[@]}"; do
  path="$(awk -v n="$name" '{ k = split($0, a, "/"); if (a[k] == n) { print; exit } }' "$work/paths.txt")"
  if [ -n "$path" ]; then echo "$path" >> "$work/selected.txt"; else echo "not in the corpus any more: $name" >&2; fi
done

git sparse-checkout init --no-cone > /dev/null
# shellcheck disable=SC2046
git sparse-checkout set --no-cone $(sed 's#^#/#' "$work/selected.txt") > /dev/null
git checkout --quiet HEAD

mkdir -p "$out"
while read -r p; do cp "$p" "$out/"; done < "$work/selected.txt"
echo "$(ls "$out" | wc -l) files in $out (Apache Tika test documents, Apache License 2.0)"
