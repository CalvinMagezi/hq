use super::*;

fn sheet(cells: &[(&str, &str)]) -> Sheet {
    let mut s = Sheet::new();
    for (at, text) in cells {
        let (c, r) = parse_cell_ref(at).unwrap();
        s.set(c, r, text);
    }
    s
}

fn at(s: &Sheet, a: &str) -> Value {
    let (c, r) = parse_cell_ref(a).unwrap();
    s.get(c, r)
}

fn n(x: f64) -> Value {
    Value::Num(x)
}

#[test]
fn the_plan_example_stock_sheet_adds_up() {
    let s = Sheet::from_table(&[
        vec!["item".into(), "qty".into(), "price".into(), "total".into()],
        vec!["Sugar".into(), "40".into(), "4200".into(), "=B2*C2".into()],
        vec!["Salt".into(), "25".into(), "1800".into(), "=B3*C3".into()],
        vec!["".into(), "".into(), "".into(), "=SUM(D2:D3)".into()],
    ]);
    assert_eq!(at(&s, "D2"), n(168000.0));
    assert_eq!(at(&s, "D3"), n(45000.0));
    assert_eq!(at(&s, "D4"), n(213000.0));
    assert_eq!(s.size(), (4, 4));
}

#[test]
fn operators_follow_spreadsheet_precedence() {
    let s = sheet(&[
        ("A1", "=1+2*3"),
        ("A2", "=(1+2)*3"),
        ("A3", "=-2^2"),
        ("A4", "=2^3^2"),
        ("A5", "=10/4"),
        ("A6", "=50%"),
        ("A7", "=1+50%"),
        ("A8", "=\"a\"&\"b\"&1"),
        ("A9", "=2*-3"),
    ]);
    let want = [7.0, 9.0, 4.0, 64.0, 2.5, 0.5, 1.5];
    for (i, w) in want.iter().enumerate() {
        assert_eq!(at(&s, &format!("A{}", i + 1)), n(*w), "A{}", i + 1);
    }
    assert_eq!(at(&s, "A8"), Value::Text("ab1".into()));
    assert_eq!(at(&s, "A9"), n(-6.0));
}

#[test]
fn comparisons_text_and_logic() {
    let s = sheet(&[
        ("A1", "5"),
        ("B1", "=A1>3"),
        ("B2", "=A1<>5"),
        ("B3", "=\"Apple\"=\"apple\""),
        ("B4", "=IF(A1>3,\"big\",\"small\")"),
        ("B5", "=IF(A1>9,1)"),
        ("B6", "=AND(A1>1,A1<10)"),
        ("B7", "=OR(A1<1,A1>10)"),
        ("B8", "=NOT(B7)"),
        ("B9", "=\"b\">\"a\""),
    ]);
    assert_eq!(at(&s, "B1"), Value::Bool(true));
    assert_eq!(at(&s, "B2"), Value::Bool(false));
    assert_eq!(at(&s, "B3"), Value::Bool(true));
    assert_eq!(at(&s, "B4"), Value::Text("big".into()));
    assert_eq!(at(&s, "B5"), Value::Bool(false));
    assert_eq!(at(&s, "B6"), Value::Bool(true));
    assert_eq!(at(&s, "B7"), Value::Bool(false));
    assert_eq!(at(&s, "B8"), Value::Bool(true));
    assert_eq!(at(&s, "B9"), Value::Bool(true));
}

#[test]
fn aggregates_skip_text_in_ranges_and_coerce_direct_arguments() {
    let s = sheet(&[
        ("A1", "1"),
        ("A2", "two"),
        ("A3", "3"),
        ("A4", ""),
        ("B1", "=SUM(A1:A4)"),
        ("B2", "=AVERAGE(A1:A4)"),
        ("B3", "=COUNT(A1:A4)"),
        ("B4", "=COUNTA(A1:A4)"),
        ("B5", "=MIN(A1:A4)"),
        ("B6", "=MAX(A1:A4,10)"),
        ("B7", "=SUM(\"3\",TRUE)"),
        ("B8", "=PRODUCT(A1,A3,2)"),
        ("B9", "=AVERAGE(A2:A2)"),
    ]);
    let want = [4.0, 2.0, 2.0, 3.0, 1.0, 10.0, 4.0, 6.0];
    for (i, w) in want.iter().enumerate() {
        assert_eq!(at(&s, &format!("B{}", i + 1)), n(*w), "B{}", i + 1);
    }
    assert_eq!(at(&s, "B9"), Value::Err(CalcError::Div0));
}

#[test]
fn rounding_is_half_away_from_zero_and_the_small_functions_work() {
    let s = sheet(&[
        ("A1", "=ROUND(2.5,0)"),
        ("A2", "=ROUND(-2.5,0)"),
        ("A3", "=ROUND(1234.5678,2)"),
        ("A4", "=ROUND(1234.5678,-2)"),
        ("A5", "=INT(-1.5)"),
        ("A6", "=MOD(-3,2)"),
        ("A7", "=ABS(-4)"),
        ("A8", "=SQRT(16)"),
        ("A9", "=LEN(\"héllo\")"),
        ("B1", "=CONCATENATE(\"a\",1,TRUE)"),
    ]);
    let want = [3.0, -3.0, 1234.57, 1200.0, -2.0, 1.0, 4.0, 4.0, 5.0];
    for (i, w) in want.iter().enumerate() {
        assert_eq!(at(&s, &format!("A{}", i + 1)), n(*w), "A{}", i + 1);
    }
    assert_eq!(at(&s, "B1"), Value::Text("a1TRUE".into()));
}

#[test]
fn errors_travel_and_iferror_catches_them() {
    let s = sheet(&[
        ("A1", "=1/0"),
        ("A2", "=A1+1"),
        ("A3", "=IFERROR(A2,-1)"),
        ("A4", "=\"x\"+1"),
        ("A5", "=NOPE(1)"),
        ("A6", "=SQRT(-1)"),
        ("A7", "=SUM(A1:A2)"),
        ("A8", "=1+"),
        ("A9", "=SUM(B1:XFD99999)"),
    ]);
    assert_eq!(at(&s, "A1"), Value::Err(CalcError::Div0));
    assert_eq!(at(&s, "A2"), Value::Err(CalcError::Div0));
    assert_eq!(at(&s, "A3"), n(-1.0));
    assert_eq!(at(&s, "A4"), Value::Err(CalcError::Value));
    assert_eq!(at(&s, "A5"), Value::Err(CalcError::Name));
    assert_eq!(at(&s, "A6"), Value::Err(CalcError::Num));
    assert_eq!(at(&s, "A7"), Value::Err(CalcError::Div0));
    assert_eq!(
        at(&s, "A8"),
        Value::Err(CalcError::Name),
        "a formula that does not parse"
    );
    assert_eq!(
        at(&s, "A9"),
        Value::Err(CalcError::Ref),
        "a range too large to read"
    );
}

#[test]
fn a_cycle_is_reported_and_does_not_loop() {
    let s = sheet(&[
        ("A1", "=B1+1"),
        ("B1", "=C1+1"),
        ("C1", "=A1+1"),
        ("D1", "=D1"),
        ("E1", "7"),
    ]);
    assert_eq!(at(&s, "A1"), Value::Err(CalcError::Circular));
    assert_eq!(at(&s, "D1"), Value::Err(CalcError::Circular));
    assert_eq!(at(&s, "E1"), n(7.0));
    assert_eq!(s.calculate().len(), 5);
}

#[test]
fn a_chain_of_dependencies_is_worked_out_in_order_from_the_bottom() {
    let mut s = Sheet::new();
    s.set(0, 0, "1");
    for r in 1..3000 {
        s.set(0, r, &format!("=A{}+1", r));
    }
    // A deep chain must not overflow the stack of the machine this runs on.
    assert_eq!(s.get(0, 2999), n(3000.0));
}

#[test]
fn numbers_print_the_way_a_spreadsheet_shows_them() {
    assert_eq!(format_number(3.0), "3");
    assert_eq!(format_number(0.1 + 0.2), "0.3");
    assert_eq!(format_number(1234.5), "1234.5");
    assert_eq!(format_number(-0.5), "-0.5");
    assert_eq!(format_number(1e21), "1000000000000000000000");
    assert_eq!(column_name(0), "A");
    assert_eq!(column_name(25), "Z");
    assert_eq!(column_name(26), "AA");
    assert_eq!(column_name(701), "ZZ");
    assert_eq!(column_name(702), "AAA");
    assert_eq!(parse_cell_ref("$AB$12"), Some((27, 11)));
    assert_eq!(parse_cell_ref("A0"), None);
    assert_eq!(parse_cell_ref("12"), None);
}

#[test]
fn unary_plus_reads_an_empty_cell_as_zero_and_keeps_text() {
    let s = sheet(&[
        ("A1", "=+B1"),
        ("A2", "=+C1"),
        ("C1", "hello"),
        ("A3", "=+5"),
    ]);
    assert_eq!(at(&s, "A1"), n(0.0));
    assert_eq!(at(&s, "A2"), Value::Text("hello".into()));
    assert_eq!(at(&s, "A3"), n(5.0));
}

#[test]
fn the_newer_and_text_functions_agree_with_a_spreadsheet() {
    let s = sheet(&[
        ("A1", "5"),
        ("B1", "=_xlfn.IFS(A1>9,\"big\",A1>3,\"mid\",TRUE,\"small\")"),
        ("B2", "=IFS(A1>9,1)"),
        ("B3", "=SWITCH(A1,1,\"one\",5,\"five\",\"other\")"),
        ("B4", "=SWITCH(A1,1,\"one\")"),
        ("B5", "=ISNUMBER(A1)"),
        ("B6", "=ISNUMBER(\"5\")"),
        ("B7", "=ISBLANK(Z9)"),
        ("B8", "=ISERROR(1/0)"),
        ("B9", "=ROUNDUP(2.1,0)"),
        ("C1", "=ROUNDUP(-2.1,0)"),
        ("C2", "=ROUNDDOWN(-2.9,0)"),
        ("C3", "=LEFT(\"hello\",2)"),
        ("C4", "=RIGHT(\"hello\",3)"),
        ("C5", "=TRIM(\"  a   b \")"),
        ("C6", "=UPPER(\"ab\")&LOWER(\"CD\")"),
    ]);
    assert_eq!(at(&s, "B1"), Value::Text("mid".into()));
    assert_eq!(at(&s, "B2"), Value::Err(CalcError::Na));
    assert_eq!(at(&s, "B3"), Value::Text("five".into()));
    assert_eq!(at(&s, "B4"), Value::Err(CalcError::Na));
    assert_eq!(at(&s, "B5"), Value::Bool(true));
    assert_eq!(at(&s, "B6"), Value::Bool(false));
    assert_eq!(at(&s, "B7"), Value::Bool(true));
    assert_eq!(at(&s, "B8"), Value::Bool(true));
    assert_eq!(at(&s, "B9"), n(3.0));
    assert_eq!(at(&s, "C1"), n(-3.0));
    assert_eq!(at(&s, "C2"), n(-2.0));
    assert_eq!(at(&s, "C3"), Value::Text("he".into()));
    assert_eq!(at(&s, "C4"), Value::Text("llo".into()));
    assert_eq!(at(&s, "C5"), Value::Text("a b".into()));
    assert_eq!(at(&s, "C6"), Value::Text("ABcd".into()));
}

#[test]
fn every_listed_function_exists_and_an_unlisted_one_does_not() {
    for f in FUNCTIONS {
        let s = sheet(&[("A1", &format!("={f}()"))]);
        assert_ne!(
            at(&s, "A1"),
            Value::Err(CalcError::Name),
            "{f} is listed but not implemented"
        );
    }
    assert_eq!(
        at(&sheet(&[("A1", "=XLOOKUP(1,2,3)")]), "A1"),
        Value::Err(CalcError::Name)
    );
}

#[test]
fn lookups_payments_and_dates_agree_with_a_spreadsheet() {
    let s = sheet(&[
        ("A1", "a"),
        ("B1", "10"),
        ("A2", "b"),
        ("B2", "20"),
        ("A3", "c"),
        ("B3", "30"),
        ("D1", "=VLOOKUP(\"b\",A1:B3,2,FALSE)"),
        ("D2", "=VLOOKUP(\"z\",A1:B3,2,FALSE)"),
        ("D3", "=INDEX(A1:B3,3,2)"),
        ("D4", "=INDEX(A1:B3,4,1)"),
        ("D5", "=ROW(B7)"),
        ("D6", "=PMT(0.01,12,-1000)"),
        ("D7", "=PMT(0,10,-1000)"),
        ("E1", "44592"),
        ("E2", "=EDATE(E1,1)"),
        ("E3", "=EDATE(E1,-2)"),
        ("E4", "=EDATE(44561,2)"),
    ]);
    assert_eq!(at(&s, "D1"), n(20.0));
    assert_eq!(at(&s, "D2"), Value::Err(CalcError::Na));
    assert_eq!(at(&s, "D3"), n(30.0));
    assert_eq!(at(&s, "D4"), Value::Err(CalcError::Ref));
    assert_eq!(at(&s, "D5"), n(7.0));
    match at(&s, "D6") {
        Value::Num(x) => assert!((x - 88.8488).abs() < 1e-3, "{x}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(at(&s, "D7"), n(100.0));
    assert_eq!(at(&s, "E2"), n(44620.0));
    assert_eq!(at(&s, "E3"), n(44530.0));
    assert_eq!(at(&s, "E4"), n(44620.0));
}

#[test]
fn a_loaded_value_keeps_its_type() {
    let mut s = Sheet::new();
    s.set_value(0, 0, Value::Text("123".into()));
    s.set_value(1, 0, Value::Err(CalcError::Div0));
    s.set(2, 0, "=ISTEXT(A1)");
    s.set(3, 0, "=ISERROR(B1)");
    s.set(4, 0, "=A1+1");
    assert_eq!(s.get(2, 0), Value::Bool(true));
    assert_eq!(s.get(3, 0), Value::Bool(true));
    assert_eq!(
        s.get(4, 0),
        n(124.0),
        "arithmetic still reads number-like text as a number"
    );
}

#[test]
fn an_error_handler_does_not_hide_what_the_engine_cannot_calculate() {
    let s = sheet(&[
        ("A1", "=XLOOKUP(1,2,3)"),
        ("B1", "=IFERROR(A1,0)"),
        ("B2", "=ISERROR(A1)"),
        ("B3", "=IFERROR(1/0,7)"),
        ("B4", "=ISERROR(1/0)"),
    ]);
    assert_eq!(at(&s, "B1"), Value::Err(CalcError::Name));
    assert_eq!(at(&s, "B2"), Value::Err(CalcError::Name));
    assert_eq!(at(&s, "B3"), n(7.0));
    assert_eq!(at(&s, "B4"), Value::Bool(true));
}

#[test]
fn edate_reproduces_the_1900_quirks() {
    let s = sheet(&[
        ("A1", "=EDATE(0,0)"),
        ("A2", "=EDATE(0,60)"),
        ("A3", "=EDATE(0,-1)"),
        ("A4", "=EDATE(31,1)"),
        ("A5", "=EDATE(61,-1)"),
    ]);
    assert_eq!(at(&s, "A1"), n(0.0));
    assert_eq!(at(&s, "A2"), n(1827.0));
    assert_eq!(at(&s, "A3"), Value::Err(CalcError::Num));
    assert_eq!(
        at(&s, "A4"),
        n(59.0),
        "31 Jan 1900 plus a month is 28 Feb 1900, serial 59"
    );
    assert_eq!(
        at(&s, "A5"),
        n(32.0),
        "1 Mar 1900 minus a month is 1 Feb 1900"
    );
}

fn book() -> Workbook {
    let mut b = Workbook::new();
    let stock = b.add_sheet("Stock").unwrap();
    let prices = b.add_sheet("Price list").unwrap();
    b.set(stock, 0, 0, "40");
    b.set(stock, 1, 0, "='Price list'!A1*A1");
    b.set(stock, 2, 0, "=SUM(price list!A1:A2)");
    b.set(stock, 3, 0, "=SUM('PRICE LIST'!A1:A2)+Stock!A1");
    b.set(prices, 0, 0, "4200");
    b.set(prices, 0, 1, "=Stock!A1");
    b.set(prices, 1, 0, "=Nowhere!A1");
    b.set(prices, 2, 0, "=A1+B2");
    b
}

#[test]
fn a_formula_reads_another_sheet_by_name() {
    let b = book();
    assert_eq!(b.get(0, 1, 0), n(168_000.0));
    assert_eq!(b.get(0, 3, 0), n(4200.0 + 40.0 + 40.0));
    assert_eq!(b.get(1, 0, 1), n(40.0), "the other sheet reads back");
}

#[test]
fn a_sheet_that_does_not_exist_cannot_be_calculated_and_an_unquoted_space_does_not_parse() {
    let b = book();
    assert_eq!(b.get(1, 1, 0), Value::Err(CalcError::Name));
    assert_eq!(b.get(0, 2, 0), Value::Err(CalcError::Name));
}

#[test]
fn a_cell_names_its_own_sheet_unless_told_otherwise() {
    let b = book();
    assert_eq!(
        b.get(1, 2, 0),
        n(4200.0),
        "A1 on Price list is 4200, B2 is empty"
    );
}

#[test]
fn sheets_that_read_each_other_in_a_loop_are_circular_not_endless() {
    let mut b = Workbook::new();
    let (a, c) = (b.add_sheet("A").unwrap(), b.add_sheet("C").unwrap());
    b.set(a, 0, 0, "=C!A1+1");
    b.set(c, 0, 0, "=A!A1+1");
    assert_eq!(b.get(a, 0, 0), Value::Err(CalcError::Circular));
    assert_eq!(b.add_sheet("a"), Some(0), "names are compared without case");
    assert!(
        !b.set(9, 0, 0, "1"),
        "a sheet that was not added takes nothing"
    );
}

#[test]
fn a_column_past_xfd_is_not_a_cell() {
    let s = sheet(&[("A1", "=SUM(B1:ZZZ9)"), ("A2", "=ZZZ1")]);
    assert_eq!(at(&s, "A1"), Value::Err(CalcError::Name));
    assert_eq!(at(&s, "A2"), Value::Err(CalcError::Name));
}

#[test]
fn every_function_the_evaluator_handles_is_in_the_public_list() {
    let source = include_str!("eval.rs");
    let body = &source[source.find("fn call(name: &str").expect("call exists")..];
    for line in body
        .lines()
        .filter(|l| l.starts_with("        \"") && l.contains("=>"))
    {
        let arm = line.split("=>").next().unwrap_or_default();
        let names = arm.split('|').map(|n| n.trim().trim_matches('"'));
        // `+` is the unary plus, an operator written as a call, not a function a person types.
        for name in names.filter(|n| n.chars().all(|c| c.is_ascii_uppercase())) {
            assert!(
                FUNCTIONS.contains(&name),
                "{name} is handled but missing from FUNCTIONS, so reports would call it unsupported"
            );
        }
    }
}

#[test]
fn a_difference_within_rounding_noise_of_zero_is_zero_and_text_dates_count_as_dates() {
    let s = sheet(&[
        ("A1", "0.1"),
        ("A2", "=A1*3"),
        ("A3", "=A2-0.3"),
        ("B1", "=\"2023-06-01\"+1"),
        ("B2", "=\"2023-6-1\"+1"),
        ("B3", "=DATE_NOT_A_THING"),
    ]);
    assert_eq!(at(&s, "A3"), n(0.0));
    assert_eq!(at(&s, "B1"), n(45079.0));
    assert_eq!(
        at(&s, "B2"),
        Value::Err(CalcError::Value),
        "only the ISO form is read as a date"
    );
}
