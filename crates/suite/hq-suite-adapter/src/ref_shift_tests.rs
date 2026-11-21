use super::*;

const INS3: RowChange = RowChange::Insert { at_row: 3, n: 1 };
const DEL3: RowChange = RowChange::Delete {
    first_row: 3,
    count: 1,
};

fn ins(f: &str) -> String {
    shift_formula(f, "S", "S", INS3)
}

fn del(f: &str) -> String {
    shift_formula(f, "S", "S", DEL3)
}

#[test]
fn points_move_when_at_or_below_the_insert() {
    assert_eq!(ins("=A2+A3+A4"), "=A2+A4+A5");
    assert_eq!(ins("=$A$3*B$4+$C5"), "=$A$4*B$5+$C6");
}

#[test]
fn a_range_grows_when_the_insert_is_inside_and_moves_when_at_its_start() {
    assert_eq!(ins("=SUM(E2:E4)"), "=SUM(E2:E5)");
    assert_eq!(ins("=SUM(E3:E4)"), "=SUM(E4:E5)");
    assert_eq!(ins("=SUM(E4:E6)"), "=SUM(E5:E7)");
    assert_eq!(ins("=SUM(E1:E2)"), "=SUM(E1:E2)");
}

#[test]
fn a_range_does_not_grow_for_an_insert_just_below_it() {
    let c = RowChange::Insert { at_row: 5, n: 2 };
    assert_eq!(shift_formula("=SUM(E2:E4)", "S", "S", c), "=SUM(E2:E4)");
    assert_eq!(shift_formula("=SUM(E2:E5)", "S", "S", c), "=SUM(E2:E7)");
}

#[test]
fn deleting_shrinks_ranges_and_breaks_points() {
    assert_eq!(del("=SUM(A2:A5)"), "=SUM(A2:A4)");
    assert_eq!(del("=SUM(A3:A5)"), "=SUM(A3:A4)");
    assert_eq!(del("=SUM(A1:A3)"), "=SUM(A1:A2)");
    assert_eq!(del("=A3+A4"), "=#REF!+A3");
    assert_eq!(del("=SUM(A3:A3)"), "=SUM(#REF!)");
    assert_eq!(del("=SUM(A3:A3)+B9"), "=SUM(#REF!)+B8");
    let two = RowChange::Delete {
        first_row: 3,
        count: 2,
    };
    assert_eq!(shift_formula("=SUM(A2:A6)", "S", "S", two), "=SUM(A2:A4)");
    assert_eq!(shift_formula("=SUM(A3:A4)", "S", "S", two), "=SUM(#REF!)");
    assert_eq!(shift_formula("=SUM(A4:A9)", "S", "S", two), "=SUM(A3:A7)");
}

#[test]
fn text_functions_and_names_are_not_cells() {
    assert_eq!(ins("=\"A5\"&A5"), "=\"A5\"&A6");
    assert_eq!(ins("=\"say \"\"A5\"\" \"&A5"), "=\"say \"\"A5\"\" \"&A6");
    assert_eq!(ins("=LOG10(A5)"), "=LOG10(A6)");
    assert_eq!(ins("=ATAN2(A5,B5)+LOG10(100)"), "=ATAN2(A6,B6)+LOG10(100)");
    assert_eq!(ins("=1.5*2E5+A9"), "=1.5*2E5+A10");
    assert_eq!(ins("=SUM(A:A)+SUM($B:$B)"), "=SUM(A:A)+SUM($B:$B)");
}

#[test]
fn only_references_to_the_edited_sheet_move() {
    // Another sheet's own cells do not move.
    assert_eq!(shift_formula("=A5", "Other", "S", INS3), "=A5");
    // A qualified reference does, in any case, quoted or not.
    assert_eq!(
        shift_formula("=s!A5+S!A2", "Other", "S", INS3),
        "=s!A6+S!A2"
    );
    assert_eq!(
        shift_formula("='Price list'!A5+Prices!A5", "Other", "price LIST", INS3),
        "='Price list'!A6+Prices!A5"
    );
    assert_eq!(
        shift_formula("=SUM(S!A2:A5)", "Other", "S", INS3),
        "=SUM(S!A2:A6)"
    );
    // On the edited sheet, a qualified reference to another sheet stays.
    assert_eq!(
        shift_formula("=Other!A5+A5", "S", "S", INS3),
        "=Other!A5+A6"
    );
    assert_eq!(
        shift_formula("='It''s'!A5", "X", "it's", INS3),
        "='It''s'!A6"
    );
    // Deleting a row another sheet reads.
    assert_eq!(
        shift_formula("=S!A3+S!A4", "Other", "S", DEL3),
        "=#REF!+S!A3"
    );
}

#[test]
fn a_sheet_called_like_a_cell_is_a_sheet() {
    assert_eq!(shift_formula("=A1!B5", "X", "S", INS3), "=A1!B5");
    assert_eq!(shift_formula("='A1'!B5", "X", "A1", INS3), "='A1'!B6");
    assert_eq!(shift_formula("='A1'!B5", "X", "S", INS3), "='A1'!B5");
}

#[test]
fn whole_rows_move_like_ranges() {
    assert_eq!(ins("=SUM(2:5)"), "=SUM(2:6)");
    assert_eq!(ins("=SUM($4:$6)"), "=SUM($5:$7)");
    assert_eq!(del("=SUM(3:3)"), "=SUM(#REF!)");
}

#[test]
fn a_reference_past_the_last_row_is_gone() {
    let c = RowChange::Insert { at_row: 1, n: 5 };
    assert_eq!(shift_formula("=A1048574", "S", "S", c), "=#REF!");
}

#[test]
fn nothing_else_changes_byte_for_byte() {
    for f in [
        "=1+2",
        "= A1 + 2",
        "=SUM(B2:B2)*0.5",
        "",
        "text with A9 in it",
        "=Other!A:A",
    ] {
        let same = shift_formula(f, "Other", "S", INS3);
        assert_eq!(same, f);
    }
    assert_eq!(ins("= A5 + 2"), "= A6 + 2");
}

#[test]
fn cross_sheet_ranges_shift_both_ends() {
    assert_eq!(
        shift_formula("=SUM(S!A2:B9)", "T", "S", INS3),
        "=SUM(S!A2:B10)"
    );
    assert_eq!(shift_formula("=S!A:A", "T", "S", INS3), "=S!A:A");
}

#[test]
fn insert_then_delete_is_identity_when_nothing_reads_the_inserted_row() {
    let formulas = [
        "=A2+B$9*$C$12",
        "=SUM(A2:A40)",
        "=SUM(A5:A9)+LOG10(B30)",
        "=IF(A20>1,\"A21\",B22)",
        "=S!A9+S!B2:B20",
    ];
    for at in 2..14u32 {
        for n in 1..4u32 {
            for f in formulas {
                // Skip a formula that points exactly at the row that is inserted and removed:
                // it is moved down and back, so it is the same too, but a range that grew by an
                // insert inside it shrinks back as well. The loop checks all of them.
                let up = shift_formula(f, "S", "S", RowChange::Insert { at_row: at, n });
                let back = shift_formula(
                    &up,
                    "S",
                    "S",
                    RowChange::Delete {
                        first_row: at,
                        count: n,
                    },
                );
                assert_eq!(back, f, "insert {n} at {at}: {up}");
            }
        }
    }
}

#[test]
fn from_ranges_move_with_their_rows() {
    assert_eq!(shift_from("B2:B4", INS3), FromShift::Moved("B2:B5".into()));
    assert_eq!(shift_from("B1:B2", INS3), FromShift::Same);
    assert_eq!(
        shift_from("[[Stock]]!A4:C6", INS3),
        FromShift::Moved("[[Stock]]!A5:C7".into())
    );
    assert_eq!(
        shift_from("[[Stock#Prices]]!A1:C6", INS3),
        FromShift::Moved("[[Stock#Prices]]!A1:C7".into())
    );
    assert_eq!(shift_from("B3:B3", DEL3), FromShift::Gone);
    assert_eq!(shift_from("B2:B5", DEL3), FromShift::Moved("B2:B4".into()));
    assert_eq!(shift_from("not a range", DEL3), FromShift::Same);
}

// ---------------------------------------------------------------- columns

const CINS2: ColChange = ColChange::Insert { at_col: 2, n: 1 };
const CDEL2: ColChange = ColChange::Delete {
    first_col: 2,
    count: 1,
};

fn cins(f: &str) -> String {
    shift_formula(f, "S", "S", CINS2)
}

fn cdel(f: &str) -> String {
    shift_formula(f, "S", "S", CDEL2)
}

#[test]
fn column_points_move_when_at_or_right_of_the_insert() {
    assert_eq!(cins("=A2+B2+C2"), "=A2+C2+D2");
    assert_eq!(cins("=$B$2*C$3+$D5"), "=$C$2*D$3+$E5");
    assert_eq!(cins("=Z1"), "=AA1");
    assert_eq!(cins("=a1+b1"), "=a1+C1");
}

#[test]
fn a_column_range_grows_inside_and_moves_at_its_start() {
    assert_eq!(cins("=SUM(A2:C2)"), "=SUM(A2:D2)");
    assert_eq!(cins("=SUM(B2:C2)"), "=SUM(C2:D2)");
    assert_eq!(cins("=SUM(C2:D2)"), "=SUM(D2:E2)");
    assert_eq!(cins("=SUM(A2:A9)"), "=SUM(A2:A9)");
    let c = ColChange::Insert { at_col: 4, n: 2 };
    assert_eq!(shift_formula("=SUM(A1:C1)", "S", "S", c), "=SUM(A1:C1)");
    assert_eq!(shift_formula("=SUM(A1:D1)", "S", "S", c), "=SUM(A1:F1)");
}

#[test]
fn deleting_a_column_shrinks_ranges_and_breaks_points() {
    assert_eq!(cdel("=SUM(A2:D2)"), "=SUM(A2:C2)");
    assert_eq!(cdel("=SUM(B2:D2)"), "=SUM(B2:C2)");
    assert_eq!(cdel("=SUM(A2:B2)"), "=SUM(A2:A2)");
    assert_eq!(cdel("=B2+C2"), "=#REF!+B2");
    assert_eq!(cdel("=SUM(B2:B9)"), "=SUM(#REF!)");
    assert_eq!(cdel("=SUM(B2:B9)+D1"), "=SUM(#REF!)+C1");
    let two = ColChange::Delete {
        first_col: 2,
        count: 2,
    };
    assert_eq!(shift_formula("=SUM(B1:C1)", "S", "S", two), "=SUM(#REF!)");
    assert_eq!(shift_formula("=SUM(C1:F1)", "S", "S", two), "=SUM(B1:D1)");
}

#[test]
fn whole_columns_move_and_whole_rows_stay() {
    assert_eq!(
        cins("=SUM(B:B)+SUM($C:$D)+SUM(A:A)"),
        "=SUM(C:C)+SUM($D:$E)+SUM(A:A)"
    );
    assert_eq!(cins("=SUM(A:C)"), "=SUM(A:D)");
    assert_eq!(cdel("=SUM(B:B)"), "=SUM(#REF!)");
    assert_eq!(cdel("=SUM(A:D)"), "=SUM(A:C)");
    assert_eq!(cins("=SUM(2:5)+B2"), "=SUM(2:5)+C2");
    // And a row change leaves whole columns alone, as before.
    assert_eq!(ins("=SUM(B:B)+SUM($C:$D)"), "=SUM(B:B)+SUM($C:$D)");
}

#[test]
fn column_changes_follow_the_sheet_names() {
    assert_eq!(shift_formula("=B2", "Other", "S", CINS2), "=B2");
    assert_eq!(
        shift_formula("=S!B2+'S'!B3", "Other", "S", CINS2),
        "=S!C2+'S'!C3"
    );
    assert_eq!(
        shift_formula("=SUM(S!B:B)", "Other", "s", CINS2),
        "=SUM(S!C:C)"
    );
    assert_eq!(
        shift_formula("=SUM(Other!B:B)", "S", "S", CINS2),
        "=SUM(Other!B:B)"
    );
    assert_eq!(shift_formula("=S!B2", "Other", "S", CDEL2), "=#REF!");
    assert_eq!(cins("=\"B2\"&LOG10(B2)"), "=\"B2\"&LOG10(C2)");
}

#[test]
fn a_reference_pushed_past_the_last_column_is_gone() {
    let c = ColChange::Insert { at_col: 1, n: 2 };
    assert_eq!(shift_formula("=XFD1", "S", "S", c), "=#REF!");
}

#[test]
fn column_insert_then_delete_is_identity() {
    let formulas = [
        "=A2+B$9*$C$12",
        "=SUM(A2:F2)",
        "=SUM(C5:E9)+LOG10(B30)",
        "=IF(A20>1,\"A21\",D22)",
        "=S!B9+S!B2:F20+SUM(S!C:D)",
    ];
    for at in 1..8u32 {
        for n in 1..4u32 {
            for f in formulas {
                let up = shift_formula(f, "S", "S", ColChange::Insert { at_col: at, n });
                let back = shift_formula(
                    &up,
                    "S",
                    "S",
                    ColChange::Delete {
                        first_col: at,
                        count: n,
                    },
                );
                assert_eq!(back, f, "insert {n} at {at}: {up}");
            }
        }
    }
}

#[test]
fn from_ranges_move_with_their_columns() {
    assert_eq!(shift_from("A2:C4", CINS2), FromShift::Moved("A2:D4".into()));
    assert_eq!(shift_from("A2:A4", CINS2), FromShift::Same);
    assert_eq!(
        shift_from("[[Stock#Prices]]!C1:D6", CINS2),
        FromShift::Moved("[[Stock#Prices]]!D1:E6".into())
    );
    assert_eq!(shift_from("B2:B4", CDEL2), FromShift::Gone);
    assert_eq!(shift_from("B2:D4", CDEL2), FromShift::Moved("B2:C4".into()));
}
