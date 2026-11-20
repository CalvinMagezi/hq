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
