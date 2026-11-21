//! Differential-style tests for the second batch of functions: each expected value is what Excel
//! documents or returns, including the error cases. Where Excel's answer cannot be reproduced the
//! test asserts `#NAME?` ("cannot calculate"), never a plausible number.

use super::*;

const NAME: Value = Value::Err(CalcError::Name);
const NA: Value = Value::Err(CalcError::Na);
const VALUE: Value = Value::Err(CalcError::Value);
const NUM: Value = Value::Err(CalcError::Num);
const DIV0: Value = Value::Err(CalcError::Div0);

fn n(x: f64) -> Value {
    Value::Num(x)
}

fn t(s: &str) -> Value {
    Value::Text(s.into())
}

/// The value of formula `f` in a sheet that also holds `cells`.
fn eval_in(cells: &[(&str, &str)], f: &str) -> Value {
    let mut s = Sheet::new();
    for (at, text) in cells {
        let (c, r) = parse_cell_ref(at).unwrap();
        s.set(c, r, text);
    }
    // Far away from any test data.
    let (c, r) = parse_cell_ref("Z99").unwrap();
    s.set(c, r, &format!("={f}"));
    s.get(c, r)
}

fn ev(f: &str) -> Value {
    eval_in(&[], f)
}

/// Names a-c with amounts, a region and a descending list.
const DATA: &[(&str, &str)] = &[
    ("A1", "a"),
    ("A2", "b"),
    ("A3", "a"),
    ("A4", "c"),
    ("A5", "a"),
    ("B1", "10"),
    ("B2", "20"),
    ("B3", "30"),
    ("B4", "40"),
    ("B5", "50"),
    ("C1", "x"),
    ("C2", "x"),
    ("C3", "y"),
    ("C4", "y"),
    ("C5", "x"),
    ("E1", "40"),
    ("E2", "30"),
    ("E3", "20"),
    ("E4", "10"),
    ("F1", "p"),
    ("F2", "q"),
    ("F3", "r"),
    ("F4", "s"),
    ("F5", "t"),
    ("H1", "a"),
    ("I1", "b"),
    ("J1", "c"),
    ("H2", "1"),
    ("I2", "2"),
    ("J2", "3"),
    ("K1", "3"),
    ("K2", "1"),
    ("K3", "4"),
    ("K4", "1"),
    ("K5", "5"),
];

fn d(f: &str) -> Value {
    eval_in(DATA, f)
}

#[test]
fn the_unsupported_functions_are_named_with_a_reason_and_never_calculated() {
    for f in [
        "TODAY",
        "NOW",
        "SUMPRODUCT",
        "CHOOSE",
        "OFFSET",
        "UNIQUE",
        "SORT",
        "FILTER",
    ] {
        assert!(!FUNCTIONS.contains(&f), "{f} must stay unsupported");
        assert!(why_unsupported(f).is_some(), "{f} has no stated reason");
    }
    assert_eq!(ev("TODAY()"), NAME);
    assert_eq!(ev("NOW()"), NAME);
    assert_eq!(d("SUMPRODUCT(B1:B5,B1:B5)"), NAME);
    assert_eq!(ev("CHOOSE(2,10,20)"), NAME);
    assert_eq!(d("SUM(UNIQUE(B1:B5))"), NAME);
    // A formula built on one of them is not calculated either, even inside an error handler.
    assert_eq!(ev("IFERROR(TODAY()+1,0)"), NAME);
    assert_eq!(why_unsupported("sum"), None);
}

#[test]
fn true_and_false_are_literals_and_functions() {
    assert_eq!(ev("TRUE"), Value::Bool(true));
    assert_eq!(ev("false"), Value::Bool(false));
    assert_eq!(ev("TRUE()"), Value::Bool(true));
    assert_eq!(ev("AND(TRUE,FALSE())"), Value::Bool(false));
    assert_eq!(ev("IF(TRUE,1,2)"), n(1.0));
    assert_eq!(ev("TRUE(1)"), VALUE);
}

#[test]
fn ifna_catches_only_not_available() {
    assert_eq!(d("IFNA(VLOOKUP(\"z\",A1:B3,2,FALSE),\"none\")"), t("none"));
    assert_eq!(ev("IFNA(1/0,\"x\")"), DIV0);
    assert_eq!(ev("IFNA(NOSUCH(1),1)"), NAME);
    assert_eq!(ev("IFNA(5,6)"), n(5.0));
}

#[test]
fn multi_criteria_functions_follow_excel() {
    assert_eq!(d("SUMIFS(B1:B5,A1:A5,\"a\")"), n(90.0));
    assert_eq!(d("SUMIFS(B1:B5,A1:A5,\"a\",C1:C5,\"x\")"), n(60.0));
    assert_eq!(d("COUNTIFS(A1:A5,\"a\",B1:B5,\">=30\")"), n(2.0));
    assert_eq!(d("COUNTIFS(A1:A5,\"a\")"), n(3.0));
    assert_eq!(d("AVERAGEIF(A1:A5,\"a\",B1:B5)"), n(30.0));
    assert_eq!(d("AVERAGEIF(B1:B5,\">20\")"), n(40.0));
    assert_eq!(d("AVERAGEIF(A1:A5,\"zz\",B1:B5)"), DIV0);
    assert_eq!(d("AVERAGEIFS(B1:B5,A1:A5,\"a\",C1:C5,\"x\")"), n(30.0));
    assert_eq!(d("MAXIFS(B1:B5,A1:A5,\"a\")"), n(50.0));
    assert_eq!(d("MINIFS(B1:B5,A1:A5,\"a\")"), n(10.0));
    // Nothing matches: MAXIFS and MINIFS give 0, as in Excel.
    assert_eq!(d("MAXIFS(B1:B5,A1:A5,\"zz\")"), n(0.0));
    assert_eq!(d("MINIFS(B1:B5,A1:A5,\"zz\")"), n(0.0));
    // Ranges of different sizes, a wrong argument count, a non-range.
    assert_eq!(d("SUMIFS(B1:B5,A1:A4,\"a\")"), VALUE);
    assert_eq!(d("SUMIFS(B1:B5,A1:A5)"), VALUE);
    assert_eq!(d("COUNTIFS(A1:A5)"), VALUE);
    assert_eq!(d("COUNTIFS(5,5)"), VALUE);
    // An error in a matching cell of the sum range travels.
    let cells = [("A1", "a"), ("B1", "=1/0")];
    assert_eq!(eval_in(&cells, "SUMIFS(B1:B1,A1:A1,\"a\")"), DIV0);
}

#[test]
fn countif_distinguishes_blank_from_zero() {
    let cells = [("D1", "0"), ("D3", "0"), ("D4", "x"), ("D5", "")];
    assert_eq!(eval_in(&cells, "COUNTIF(D1:D5,0)"), n(2.0));
    assert_eq!(eval_in(&cells, "COUNTIF(D1:D4,\"<>\")"), n(3.0));
    assert_eq!(eval_in(&cells, "COUNTIF(D1:D4,\"=\")"), n(1.0));
    assert_eq!(eval_in(&cells, "COUNTIF(D1:D4,\"\")"), n(1.0));
}

#[test]
fn median_large_small_rank() {
    assert_eq!(d("MEDIAN(K1:K5)"), n(3.0));
    assert_eq!(d("MEDIAN(K1:K4)"), n(2.0));
    assert_eq!(d("MEDIAN(K1:K5,100)"), n(3.5));
    assert_eq!(d("MEDIAN(G1:G3)"), NUM);
    assert_eq!(d("LARGE(K1:K5,2)"), n(4.0));
    assert_eq!(d("SMALL(K1:K5,2)"), n(1.0));
    assert_eq!(d("SMALL(K1:K5,5)"), n(5.0));
    assert_eq!(d("LARGE(K1:K5,6)"), NUM);
    assert_eq!(d("LARGE(K1:K5,0)"), NUM);
    // How Excel rounds a fractional k is not reproduced.
    assert_eq!(d("LARGE(K1:K5,1.5)"), NAME);
    assert_eq!(d("RANK(4,K1:K5)"), n(2.0));
    assert_eq!(d("RANK(1,K1:K5)"), n(4.0));
    assert_eq!(d("RANK(1,K1:K5,1)"), n(1.0));
    assert_eq!(d("RANK.EQ(5,K1:K5,0)"), n(1.0));
    assert_eq!(d("RANK(2,K1:K5)"), NA);
    assert_eq!(d("RANK(2,7)"), VALUE);
}

#[test]
fn match_follows_excel_and_refuses_what_it_cannot_reproduce() {
    assert_eq!(d("MATCH(30,B1:B5,0)"), n(3.0));
    assert_eq!(d("MATCH(30,B1:B5)"), n(3.0));
    assert_eq!(d("MATCH(35,B1:B5,1)"), n(3.0));
    assert_eq!(d("MATCH(5,B1:B5,1)"), NA);
    assert_eq!(d("MATCH(99,B1:B5,0)"), NA);
    assert_eq!(d("MATCH(\"c\",A1:A5,0)"), n(4.0));
    assert_eq!(d("MATCH(\"C\",A1:A5,0)"), n(4.0));
    assert_eq!(d("MATCH(\"a*\",A1:A5,0)"), n(1.0));
    assert_eq!(d("MATCH(\"?\",A1:A5,0)"), n(1.0));
    // A number never matches a text.
    assert_eq!(d("MATCH(\"30\",B1:B5,0)"), NA);
    assert_eq!(d("MATCH(25,E1:E4,-1)"), n(2.0));
    assert_eq!(d("MATCH(5,E1:E4,-1)"), n(4.0));
    assert_eq!(d("MATCH(50,E1:E4,-1)"), NA);
    // A table is not a vector.
    assert_eq!(d("MATCH(1,A1:B3,0)"), NA);
    // Approximate matches over data that is not sorted, or of another kind, are not guessed.
    assert_eq!(d("MATCH(25,B1:B5,-1)"), NAME);
    assert_eq!(d("MATCH(\"b\",A1:A5,1)"), NAME);
    assert_eq!(d("MATCH(35,A1:A5,1)"), NAME);
    assert_eq!(d("MATCH(2,K1:K5,1)"), NAME);
    assert_eq!(d("MATCH(2,G1:G3,0)"), NA);
    assert_eq!(d("MATCH(\"a~*\",A1:A5,0)"), NAME);
}

#[test]
fn lookup_and_hlookup() {
    assert_eq!(d("LOOKUP(35,B1:B5,F1:F5)"), t("r"));
    assert_eq!(d("LOOKUP(100,B1:B5,F1:F5)"), t("t"));
    assert_eq!(d("LOOKUP(5,B1:B5,F1:F5)"), NA);
    assert_eq!(d("LOOKUP(35,B1:B5)"), n(30.0));
    assert_eq!(d("LOOKUP(2,A1:B3)"), NAME);
    assert_eq!(d("LOOKUP(35,K1:K5,F1:F5)"), NAME);
    assert_eq!(d("HLOOKUP(\"b\",H1:J2,2,FALSE)"), n(2.0));
    assert_eq!(d("HLOOKUP(\"z\",H1:J2,2,FALSE)"), NA);
    assert_eq!(d("HLOOKUP(\"b\",H1:J2,3,0)"), Value::Err(CalcError::Ref));
    assert_eq!(d("HLOOKUP(\"bb\",H1:J2,2,TRUE)"), n(2.0));
    assert_eq!(d("HLOOKUP(\"b\",H1:J2,0,0)"), VALUE);
}

#[test]
fn text_functions() {
    assert_eq!(ev("VALUE(\"1,234.5\")"), n(1234.5));
    assert_eq!(ev("VALUE(\"-1,234\")"), n(-1234.0));
    assert_eq!(ev("VALUE(\"50%\")"), n(0.5));
    assert_eq!(ev("VALUE(\" 12 \")"), n(12.0));
    assert_eq!(ev("VALUE(\"1E3\")"), n(1000.0));
    assert_eq!(ev("VALUE(\"2024-03-15\")"), n(45366.0));
    assert_eq!(ev("VALUE(7)"), n(7.0));
    assert_eq!(ev("VALUE(\"abc\")"), VALUE);
    assert_eq!(ev("VALUE(\"\")"), VALUE);
    assert_eq!(ev("VALUE(\"inf\")"), VALUE);
    assert_eq!(ev("VALUE(\"nan\")"), VALUE);
    assert_eq!(ev("VALUE(TRUE)"), VALUE);
    // Currency signs, odd grouping, other date and time styles: not read, not guessed.
    assert_eq!(ev("VALUE(\"$12\")"), NAME);
    assert_eq!(ev("VALUE(\"1,23\")"), NAME);
    assert_eq!(ev("VALUE(\"3/15/2024\")"), NAME);
    assert_eq!(ev("VALUE(\"12:30\")"), NAME);

    assert_eq!(ev("REPT(\"ab\",3)"), t("ababab"));
    assert_eq!(ev("REPT(\"a\",0)"), t(""));
    assert_eq!(ev("REPT(\"a\",2.9)"), t("aa"));
    assert_eq!(ev("REPT(\"a\",-1)"), VALUE);
    assert_eq!(ev("REPT(\"abc\",20000)"), VALUE);

    assert_eq!(
        ev("PROPER(\"hello wORLD o'neil 2nd-try\")"),
        t("Hello World O'Neil 2Nd-Try")
    );
    assert_eq!(ev("PROPER(\"\")"), t(""));

    assert_eq!(ev("MID(\"abcdef\",2,3)"), t("bcd"));
    assert_eq!(ev("MID(\"abc\",2,100)"), t("bc"));
    assert_eq!(ev("MID(\"abc\",5,2)"), t(""));
    assert_eq!(ev("MID(\"abc\",0,1)"), VALUE);
    assert_eq!(ev("MID(\"abc\",2,-1)"), VALUE);
    assert_eq!(ev("MID(12345,2,2)"), t("23"));

    assert_eq!(ev("EXACT(\"a\",\"A\")"), Value::Bool(false));
    assert_eq!(ev("EXACT(\"a\",\"a\")"), Value::Bool(true));
    assert_eq!(ev("EXACT(1,\"1\")"), Value::Bool(true));
}

#[test]
fn dates_follow_excel_including_1900() {
    assert_eq!(ev("DATE(2024,3,15)"), n(45366.0));
    assert_eq!(ev("DATE(2024,14,1)"), n(45689.0));
    assert_eq!(ev("DATE(2024,1,0)"), n(45291.0));
    assert_eq!(ev("DATE(2024,3,32)"), n(45383.0));
    assert_eq!(ev("DATE(2024,-1,1)"), n(45231.0));
    assert_eq!(ev("DATE(1900,1,1)"), n(1.0));
    assert_eq!(ev("DATE(1900,1,0)"), n(0.0));
    assert_eq!(ev("DATE(1900,2,29)"), n(60.0));
    assert_eq!(ev("DATE(1900,3,0)"), n(60.0));
    assert_eq!(ev("DATE(1900,3,1)"), n(61.0));
    // Years 0 to 1899 count from 1900.
    assert_eq!(ev("DATE(100,1,1)"), ev("DATE(2000,1,1)"));
    assert_eq!(ev("DATE(-1,1,1)"), NUM);
    assert_eq!(ev("DATE(10000,1,1)"), NUM);
    assert_eq!(ev("DATE(9999,12,31)"), n(2_958_465.0));
    assert_eq!(ev("DATE(1,2)"), VALUE);

    assert_eq!(ev("YEAR(45366)"), n(2024.0));
    assert_eq!(ev("MONTH(45366)"), n(3.0));
    assert_eq!(ev("DAY(45366)"), n(15.0));
    assert_eq!(ev("DAY(45366.99)"), n(15.0));
    assert_eq!(ev("YEAR(0)"), n(1900.0));
    assert_eq!(ev("DAY(0)"), n(0.0));
    assert_eq!(ev("MONTH(60)"), n(2.0));
    assert_eq!(ev("DAY(60)"), n(29.0));
    assert_eq!(ev("YEAR(\"2024-03-15\")"), n(2024.0));
    assert_eq!(ev("YEAR(-1)"), NUM);
    assert_eq!(ev("YEAR(abc)"), NAME);
    assert_eq!(ev("YEAR(\"abc\")"), VALUE);
    // A date text in another style might be a date in Excel: not read.
    assert_eq!(ev("YEAR(\"15/03/2024\")"), NAME);
}

#[test]
fn weekday_matches_excels_calendar() {
    // 15 March 2024 was a Friday.
    assert_eq!(ev("WEEKDAY(45366)"), n(6.0));
    assert_eq!(ev("WEEKDAY(45366,1)"), n(6.0));
    assert_eq!(ev("WEEKDAY(45366,2)"), n(5.0));
    assert_eq!(ev("WEEKDAY(45366,3)"), n(4.0));
    assert_eq!(ev("WEEKDAY(45366,11)"), n(5.0));
    assert_eq!(ev("WEEKDAY(45366,12)"), n(4.0));
    assert_eq!(ev("WEEKDAY(45366,17)"), n(6.0));
    assert_eq!(ev("WEEKDAY(45366,9)"), NUM);
    // Day 1 is a Sunday in Excel, and day 0 a Saturday.
    assert_eq!(ev("WEEKDAY(1)"), n(1.0));
    assert_eq!(ev("WEEKDAY(0)"), n(7.0));
    assert_eq!(ev("WEEKDAY(61)"), n(5.0));
}

#[test]
fn eomonth_and_datedif() {
    assert_eq!(ev("EOMONTH(45366,0)"), n(45382.0));
    assert_eq!(ev("EOMONTH(45366,1)"), n(45412.0));
    assert_eq!(ev("EOMONTH(45366,-3)"), n(45291.0));
    assert_eq!(ev("EOMONTH(DATE(2024,1,31),1)"), n(45351.0));
    assert_eq!(ev("EOMONTH(DATE(2023,1,31),1)"), n(44985.0));
    assert_eq!(ev("EOMONTH(0,0)"), n(31.0));
    assert_eq!(ev("EOMONTH(45366,-2000)"), NUM);
    assert_eq!(ev("EOMONTH(45366)"), VALUE);

    let (a, b) = ("DATE(2020,1,15)", "DATE(2024,3,10)");
    assert_eq!(ev(&format!("DATEDIF({a},{b},\"Y\")")), n(4.0));
    assert_eq!(ev(&format!("DATEDIF({a},{b},\"M\")")), n(49.0));
    assert_eq!(ev(&format!("DATEDIF({a},{b},\"YM\")")), n(1.0));
    assert_eq!(ev(&format!("DATEDIF({a},{b},\"y\")")), n(4.0));
    assert_eq!(ev("DATEDIF(45366,45367,\"D\")"), n(1.0));
    assert_eq!(ev("DATEDIF(DATE(2024,1,31),DATE(2024,2,28),\"M\")"), n(0.0));
    assert_eq!(ev("DATEDIF(DATE(2024,1,31),DATE(2024,2,29),\"M\")"), n(0.0));
    assert_eq!(ev("DATEDIF(DATE(2024,1,31),DATE(2024,3,1),\"M\")"), n(1.0));
    assert_eq!(ev("DATEDIF(45367,45366,\"D\")"), NUM);
    assert_eq!(ev("DATEDIF(45366,45367,\"Q\")"), NUM);
    // Excel's "MD" and "YD" are known to misbehave: not reproduced.
    assert_eq!(ev("DATEDIF(45366,45500,\"MD\")"), NAME);
    assert_eq!(ev("DATEDIF(45366,45500,\"YD\")"), NAME);
}

#[test]
fn text_formats_for_dates_numbers_and_money() {
    let f = |v: &str, fmt: &str| ev(&format!("TEXT({v},\"{fmt}\")"));
    assert_eq!(f("45366", "yyyy-mm-dd"), t("2024-03-15"));
    assert_eq!(f("45366", "dd/mm/yyyy"), t("15/03/2024"));
    assert_eq!(f("45366", "dd.mm.yyyy"), t("15.03.2024"));
    assert_eq!(f("45366", "mmm yyyy"), t("Mar 2024"));
    assert_eq!(f("45366", "d mmmm"), t("15 March"));
    assert_eq!(f("45366", "dddd"), t("Friday"));
    assert_eq!(f("45366", "ddd, d mmm yy"), t("Fri, 15 Mar 24"));
    assert_eq!(f("45366", "m/d/yy"), t("3/15/24"));
    assert_eq!(f("45366.75", "yyyy-mm-dd"), t("2024-03-15"));
    assert_eq!(f("0", "yyyy-mm-dd"), t("1900-01-00"));
    assert_eq!(f("60", "yyyy-mm-dd"), t("1900-02-29"));
    assert_eq!(f("-1", "yyyy-mm-dd"), VALUE);
    assert_eq!(f("\"2024-03-15\"", "d mmm"), t("15 Mar"));
    // Times, and a date format with several sections, are not reproduced.
    assert_eq!(f("45366", "hh:mm"), NAME);
    assert_eq!(f("45366", "yyyy-mm-dd hh:mm"), NAME);
    assert_eq!(f("45366", "yyy"), NAME);

    assert_eq!(f("1234.5", "#,##0.00"), t("1,234.50"));
    assert_eq!(f("1234.5", "#,##0"), t("1,235"));
    assert_eq!(f("-1234.5", "#,##0.00"), t("-1,234.50"));
    assert_eq!(f("1234.5", "$#,##0.00"), t("$1,234.50"));
    assert_eq!(f("-1234.5", "$#,##0.00"), t("-$1,234.50"));
    assert_eq!(f("1234.5", "€#,##0.00"), t("€1,234.50"));
    assert_eq!(
        ev("TEXT(1234567,\"\"\"UGX \"\"#,##0\")"),
        t("UGX 1,234,567")
    );
    assert_eq!(
        ev("TEXT(1234567,\"#,##0 \"\"UGX\"\"\")"),
        t("1,234,567 UGX")
    );
    assert_eq!(f("1234567", "\\U\\G\\X #,##0"), t("UGX 1,234,567"));
    assert_eq!(f("0.256", "0.0%"), t("25.6%"));
    assert_eq!(f("0.5", "0%"), t("50%"));
    assert_eq!(f("1.005", "0.00"), t("1.01"));
    assert_eq!(f("2", "0.##"), t("2."));
    assert_eq!(f("1.5", "0.##"), t("1.5"));
    assert_eq!(f("1.25", "0.#"), t("1.3"));
    assert_eq!(f("5", "000"), t("005"));
    assert_eq!(f("0", "#,##0"), t("0"));
    assert_eq!(f("0.5", "#.##"), t(".5"));
    assert_eq!(f("1234567.891", "#,##0.0"), t("1,234,567.9"));
    assert_eq!(f("5", "0_)"), t("5 "));
    assert_eq!(f("-5", "#,##0;(#,##0)"), t("(5)"));
    assert_eq!(f("5", "#,##0;(#,##0)"), t("5"));
    assert_eq!(ev("TEXT(0,\"0;-0;\"\"zero\"\"\")"), t("zero"));
    assert_eq!(f("12", "General"), t("12"));
    assert_eq!(f("12", "@"), t("12"));
    // Not reproduced: scaling commas, scientific, colours, currency tags, fractions, unquoted
    // letters, fill characters, and a small negative that shows as zero.
    assert_eq!(f("1234", "0,"), NAME);
    assert_eq!(f("5", "0.00E+00"), NAME);
    assert_eq!(f("5", "[Red]0"), NAME);
    assert_eq!(f("5", "[$USD]0"), NAME);
    assert_eq!(f("5", "# ?/?"), NAME);
    assert_eq!(f("5", "UGX 0"), NAME);
    assert_eq!(f("5", "*-0"), NAME);
    assert_eq!(f("-0.001", "0.00"), NAME);
    assert_eq!(f("0.5", "0/0"), NAME);
    // A text that is not a number comes back as it is; a truth value shows as its name.
    assert_eq!(ev("TEXT(\"abc\",\"0.00\")"), t("abc"));
    assert_eq!(ev("TEXT(TRUE,\"0\")"), t("TRUE"));
    assert_eq!(ev("TEXT(1/0,\"0\")"), DIV0);
    assert_eq!(ev("TEXT(5,5)"), VALUE);
}

#[test]
fn rounding_works_on_the_fifteen_digits_excel_keeps() {
    assert_eq!(ev("ROUND(1.005,2)"), n(1.01));
    assert_eq!(ev("ROUND(2.675,2)"), n(2.68));
    assert_eq!(ev("ROUND(2.5,0)"), n(3.0));
    assert_eq!(ev("ROUND(-2.5,0)"), n(-3.0));
    assert_eq!(ev("ROUND(0.4,0)"), n(0.0));
    assert_eq!(ev("ROUND(0.6,0)"), n(1.0));
    assert_eq!(ev("ROUND(1234.5678,-2)"), n(1200.0));
    assert_eq!(ev("ROUND(5,-1)"), n(10.0));
    assert_eq!(ev("ROUND(4,-1)"), n(0.0));
    assert_eq!(ev("ROUND(0.05,-2)"), n(0.0));
    assert_eq!(ev("ROUNDUP(2.3*3,0)"), n(7.0));
    assert_eq!(ev("ROUNDUP(-3.14259,3)"), n(-3.143));
    assert_eq!(ev("ROUNDUP(0.0001,2)"), n(0.01));
    assert_eq!(ev("ROUNDUP(5,-1)"), n(10.0));
    assert_eq!(ev("ROUNDUP(3,0)"), n(3.0));
    assert_eq!(ev("ROUNDDOWN(6.9,0)"), n(6.0));
    assert_eq!(ev("ROUNDDOWN(-3.9,0)"), n(-3.0));
    assert_eq!(ev("ROUNDDOWN(0.0001,2)"), n(0.0));
    assert_eq!(ev("ROUNDDOWN(1234.5678,-2)"), n(1200.0));
    assert_eq!(ev("TRUNC(8.9)"), n(8.0));
    assert_eq!(ev("TRUNC(-8.9)"), n(-8.0));
    assert_eq!(ev("TRUNC(3.17159,2)"), n(3.17));
    assert_eq!(ev("SIGN(-4)"), n(-1.0));
    assert_eq!(ev("SIGN(0)"), n(0.0));
    assert_eq!(ev("SIGN(9)"), n(1.0));
    assert_eq!(ev("CEILING(2.5,1)"), n(3.0));
    assert_eq!(ev("CEILING(2.5,0.1)"), n(2.5));
    assert_eq!(ev("CEILING(-2.5,-2)"), n(-4.0));
    assert_eq!(ev("CEILING(-2.5,2)"), n(-2.0));
    assert_eq!(ev("CEILING(2.5,-2)"), NUM);
    assert_eq!(ev("CEILING(2.5,0)"), n(0.0));
    assert_eq!(ev("FLOOR(2.5,2)"), n(2.0));
    assert_eq!(ev("FLOOR(-2.5,-2)"), n(-2.0));
    assert_eq!(ev("FLOOR(-2.5,2)"), n(-4.0));
    assert_eq!(ev("FLOOR(2.5,-2)"), NUM);
    assert_eq!(ev("FLOOR(2.5,0)"), DIV0);
    assert_eq!(ev("FLOOR(3.7)"), VALUE);
}

#[test]
fn text_that_looks_like_infinity_is_not_a_number() {
    assert_eq!(ev("\"inf\"+1"), VALUE);
    assert_eq!(ev("\"nan\"*2"), VALUE);
    assert_eq!(ev("\"1e2\"+1"), n(101.0));
}

#[test]
fn whole_column_ranges_read_only_the_rows_in_use() {
    let cells = [
        ("A1", "item"),
        ("A2", "a"),
        ("A3", "b"),
        ("A4", "a"),
        ("B1", "qty"),
        ("B2", "10"),
        ("B3", "20"),
        ("B4", "30"),
    ];
    let w = |f: &str| eval_in(&cells, f);
    assert_eq!(w("SUM($B:$B)"), n(60.0));
    assert_eq!(w("SUM(B:B)"), n(60.0));
    assert_eq!(w("SUM($B:$C)"), n(60.0));
    assert_eq!(w("COUNT(B:B)"), n(3.0));
    assert_eq!(w("COUNTA(A:A)"), n(4.0));
    assert_eq!(w("MAX(B:B)"), n(30.0));
    assert_eq!(w("SUMIF(A:A,\"a\",B:B)"), n(40.0));
    assert_eq!(w("COUNTIF(A:A,\"a\")"), n(2.0));
    assert_eq!(w("SUMIFS(B:B,A:A,\"a\")"), n(40.0));
    assert_eq!(w("COUNTIF(A:A,\"<>\")"), n(4.0));
    assert_eq!(w("MATCH(\"b\",A:A,0)"), n(3.0));
    assert_eq!(w("VLOOKUP(\"b\",A:B,2,FALSE)"), n(20.0));
    assert_eq!(w("VLOOKUP(\"zz\",A:B,2,FALSE)"), NA);
    assert_eq!(w("INDEX(B:B,3)"), n(20.0));
    assert_eq!(w("INDEX(B:B,500000)"), Value::Empty);
    assert_eq!(w("INDEX(B:B,2000000)"), Value::Err(CalcError::Ref));
    assert_eq!(w("ROWS(A:A)"), n(1_048_576.0));
    assert_eq!(w("ROW(A:A)"), n(1.0));
    assert_eq!(w("SUM(B2:B1048576)"), n(60.0));
    // Counting blanks in a column cut back to the rows in use would be wrong, so it is refused.
    assert_eq!(w("COUNTIF(A:A,\"\")"), NAME);
    assert_eq!(w("COUNTIF(A:A,\"=\")"), NAME);
    assert_eq!(w("COUNTIFS(A:A,\"\")"), NAME);
    // Different shapes are still different shapes.
    assert_eq!(w("SUMIFS(B:B,A2:A4,\"a\")"), VALUE);
    // A formula inside the column it sums is a cycle, as in Excel.
    let mut s = Sheet::new();
    s.set(0, 0, "1");
    s.set(0, 1, "=SUM(A:A)");
    assert_eq!(s.get(0, 1), Value::Err(CalcError::Circular));
}

#[test]
fn whole_column_ranges_in_a_workbook_stay_in_their_sheet() {
    let mut wb = Workbook::new();
    let a = wb.add_sheet("Data").unwrap();
    let b = wb.add_sheet("Sum").unwrap();
    wb.set(a, 1, 0, "5");
    wb.set(a, 1, 1, "6");
    wb.set(b, 1, 0, "100");
    wb.set(b, 0, 0, "=SUM(Data!B:B)");
    wb.set(b, 0, 1, "=SUM(B:B)");
    assert_eq!(wb.get(b, 0, 0), n(11.0));
    assert_eq!(wb.get(b, 0, 1), n(100.0));
}

#[test]
fn a_whole_column_over_a_large_sheet_stays_fast() {
    let mut s = Sheet::new();
    for r in 0..20_000u32 {
        s.set(0, r, &r.to_string());
    }
    s.set(1, 0, "=SUM(A:A)");
    s.set(1, 1, "=COUNTIF(A:A,\">=10000\")");
    assert_eq!(s.get(1, 0), n(199_990_000.0));
    assert_eq!(s.get(1, 1), n(10_000.0));
}

#[test]
fn table_references_are_not_calculated() {
    assert_eq!(ev("SUM(Table1[Amount])"), NAME);
    assert_eq!(ev("Table1[[#This Row],[Amount]]*2"), NAME);
}
