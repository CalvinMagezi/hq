//! Native charts: one drawing part per sheet that has charts, one chart part per chart.
//!
//! A chart reads its labels and values from cells of its sheet (so editing the cells in Excel moves
//! the chart), and carries a cached copy of them so a viewer that does not recalculate still draws
//! it. Only a plain range maps to a chart this way: a chart that filters rows or groups them has no
//! single range behind it, so the caller leaves it out and says so.

use hq_suite_calc::column_name;

const CHART_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const DRAW_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const SPREAD_DRAW_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// Series colours, in order: distinguishable for the common kinds of colour blindness and readable
/// on white. Written explicitly so every viewer draws the same chart.
const PALETTE: [&str; 6] = ["2E6F9E", "D97706", "2F855A", "B83280", "6B46C1", "718096"];

/// Most series one chart carries; Excel takes more, but a chart with more is not a readable one.
pub const MAX_SERIES: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartKind {
    Bar,
    Line,
    Pie,
    Donut,
}

/// One series: the column its values sit in (zero-based) and their cached numbers.
#[derive(Debug, Clone)]
pub struct ChartSeries {
    pub name: String,
    pub col: u32,
    pub values: Vec<f64>,
}

/// A chart over a block of rows of one sheet: the header is row 1, data starts at `first_row`.
#[derive(Debug, Clone)]
pub struct ChartSpec {
    /// Index of the sheet in the list given to the writer.
    pub sheet: usize,
    pub kind: ChartKind,
    pub title: Option<String>,
    /// The column holding the labels (zero-based).
    pub x_col: u32,
    pub labels: Vec<String>,
    pub series: Vec<ChartSeries>,
    /// One-based row of the first data row.
    pub first_row: usize,
    pub x_title: Option<String>,
    pub y_title: Option<String>,
}

impl ChartSpec {
    /// Whether the writer can draw it: labels, at least one series, every series as long as the labels.
    pub fn is_drawable(&self) -> bool {
        !self.labels.is_empty()
            && !self.series.is_empty()
            && self.series.len() <= MAX_SERIES
            && self
                .series
                .iter()
                .all(|s| s.values.len() == self.labels.len())
            && !(matches!(self.kind, ChartKind::Pie | ChartKind::Donut) && self.series.len() != 1)
    }
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// A sheet name as a formula reference writes it.
fn quoted(sheet: &str) -> String {
    format!("'{}'", sheet.replace('\'', "''"))
}

fn range(sheet: &str, col: u32, first: usize, last: usize) -> String {
    let c = column_name(col);
    format!("{}!${c}${first}:${c}${last}", quoted(sheet))
}

fn title_xml(text: &str) -> String {
    format!(
        r#"<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx><c:overlay val="0"/></c:title>"#,
        esc(text)
    )
}

fn series_xml(spec: &ChartSpec, sheet: &str, i: usize) -> String {
    let s = &spec.series[i];
    let last = spec.first_row + spec.labels.len() - 1;
    let mut cats = format!(
        r#"<c:cat><c:strRef><c:f>{}</c:f><c:strCache><c:ptCount val="{}"/>"#,
        esc(&range(sheet, spec.x_col, spec.first_row, last)),
        spec.labels.len()
    );
    for (n, l) in spec.labels.iter().enumerate() {
        cats.push_str(&format!(r#"<c:pt idx="{n}"><c:v>{}</c:v></c:pt>"#, esc(l)));
    }
    cats.push_str("</c:strCache></c:strRef></c:cat>");
    let mut vals = format!(
        r#"<c:val><c:numRef><c:f>{}</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="{}"/>"#,
        esc(&range(sheet, s.col, spec.first_row, last)),
        s.values.len()
    );
    for (n, v) in s.values.iter().enumerate() {
        vals.push_str(&format!(r#"<c:pt idx="{n}"><c:v>{v}</c:v></c:pt>"#));
    }
    vals.push_str("</c:numCache></c:numRef></c:val>");
    // The series name points at the header cell of its column.
    let name = format!(
        r#"<c:tx><c:strRef><c:f>{}</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>{}</c:v></c:pt></c:strCache></c:strRef></c:tx>"#,
        esc(&range(sheet, s.col, 1, 1)),
        esc(&s.name)
    );
    let color = PALETTE[i % PALETTE.len()];
    let fill = format!(r#"<a:solidFill><a:srgbClr val="{color}"/></a:solidFill>"#);
    let paint = match spec.kind {
        ChartKind::Line => {
            format!(r#"<c:spPr><a:ln w="28575" cap="rnd">{fill}<a:round/></a:ln></c:spPr><c:marker><c:symbol val="circle"/><c:size val="6"/><c:spPr>{fill}</c:spPr></c:marker>"#)
        }
        ChartKind::Bar => format!(r#"<c:spPr>{fill}</c:spPr><c:invertIfNegative val="0"/>"#),
        // A pie or donut colours each slice, not the series.
        ChartKind::Pie | ChartKind::Donut => (0..spec.labels.len())
            .map(|n| {
                let c = PALETTE[n % PALETTE.len()];
                format!(
                    r#"<c:dPt><c:idx val="{n}"/><c:bubble3D val="0"/><c:spPr><a:solidFill><a:srgbClr val="{c}"/></a:solidFill><a:ln w="12700"><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></a:ln></c:spPr></c:dPt>"#
                )
            })
            .collect(),
    };
    format!(r#"<c:ser><c:idx val="{i}"/><c:order val="{i}"/>{name}{paint}{cats}{vals}</c:ser>"#)
}

fn axis_xml(spec: &ChartSpec) -> String {
    let cat_title = spec.x_title.as_deref().map(title_xml).unwrap_or_default();
    let val_title = spec.y_title.as_deref().map(title_xml).unwrap_or_default();
    format!(
        r#"<c:catAx><c:axId val="111"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/>{cat_title}<c:numFmt formatCode="General" sourceLinked="0"/><c:tickLblPos val="nextTo"/><c:crossAx val="222"/><c:crosses val="autoZero"/><c:auto val="1"/><c:lblAlgn val="ctr"/><c:lblOffset val="100"/></c:catAx><c:valAx><c:axId val="222"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines/>{val_title}<c:numFmt formatCode="General" sourceLinked="1"/><c:tickLblPos val="nextTo"/><c:crossAx val="111"/><c:crosses val="autoZero"/></c:valAx>"#
    )
}

/// The chart part for `spec`, whose data lives on the sheet called `sheet`.
pub(crate) fn chart_xml(spec: &ChartSpec, sheet: &str) -> String {
    let series: String = (0..spec.series.len())
        .map(|i| series_xml(spec, sheet, i))
        .collect();
    let plot = match spec.kind {
        ChartKind::Bar => format!(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>{series}<c:axId val="111"/><c:axId val="222"/></c:barChart>{}"#,
            axis_xml(spec)
        ),
        ChartKind::Line => format!(
            r#"<c:lineChart><c:grouping val="standard"/><c:varyColors val="0"/>{series}<c:marker val="1"/><c:axId val="111"/><c:axId val="222"/></c:lineChart>{}"#,
            axis_xml(spec)
        ),
        ChartKind::Pie => {
            format!(r#"<c:pieChart><c:varyColors val="1"/>{series}</c:pieChart>"#)
        }
        ChartKind::Donut => format!(
            r#"<c:doughnutChart><c:varyColors val="1"/>{series}<c:holeSize val="55"/></c:doughnutChart>"#
        ),
    };
    let title = match &spec.title {
        Some(t) => format!("{}<c:autoTitleDeleted val=\"0\"/>", title_xml(t)),
        None => r#"<c:autoTitleDeleted val="1"/>"#.to_string(),
    };
    let legend = if spec.series.len() > 1 || matches!(spec.kind, ChartKind::Pie | ChartKind::Donut)
    {
        r#"<c:legend><c:legendPos val="b"/><c:overlay val="0"/></c:legend>"#
    } else {
        ""
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="{CHART_NS}" xmlns:a="{DRAW_NS}" xmlns:r="{REL_NS}"><c:roundedCorners val="0"/><c:chart>{title}<c:plotArea><c:layout/>{plot}</c:plotArea>{legend}<c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart></c:chartSpace>"#
    )
}

/// The drawing part for one sheet: each chart is placed to the right of the data, stacked down.
/// `columns` is how many columns the sheet's data uses; `ids` are the relationship ids, one per chart.
pub(crate) fn drawing_xml(columns: usize, count: usize) -> String {
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="{SPREAD_DRAW_NS}" xmlns:a="{DRAW_NS}">"#
    );
    let left = columns + 1;
    for i in 0..count {
        let top = i * 20;
        s.push_str(&format!(
            r#"<xdr:twoCellAnchor><xdr:from><xdr:col>{left}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{top}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>{}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="{}" name="Chart {}"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="{CHART_NS}"><c:chart xmlns:c="{CHART_NS}" xmlns:r="{REL_NS}" r:id="rId{}"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>"#,
            left + 8,
            top + 18,
            i + 2,
            i + 1,
            i + 1
        ));
    }
    s.push_str("</xdr:wsDr>");
    s
}

/// The relationships of a drawing: `rId{n}` points at `../charts/chart{first + n - 1}.xml`.
pub(crate) fn drawing_rels(first_chart: usize, count: usize) -> String {
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{PKG_REL_NS}">"#
    );
    for n in 1..=count {
        s.push_str(&format!(
            r#"<Relationship Id="rId{n}" Type="{REL_NS}/chart" Target="../charts/chart{}.xml"/>"#,
            first_chart + n - 1
        ));
    }
    s.push_str("</Relationships>");
    s
}
