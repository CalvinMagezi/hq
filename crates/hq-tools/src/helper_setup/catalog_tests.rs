//! Catalog checks that need the harness table, so they live above the free catalog module.

use crate::helper_catalog::CATALOG;

#[test]
fn every_helper_is_a_harness_hq_can_start() {
    for h in CATALOG {
        let spec = crate::harness_session::spec::SPECS
            .iter()
            .find(|s| s.harness == h.id)
            .unwrap_or_else(|| panic!("{} is not a known harness", h.id));
        assert!(
            h.programs.contains(&spec.binary()),
            "{}: HQ starts `{}`, which the catalog must look for",
            h.id,
            spec.binary()
        );
    }
}
