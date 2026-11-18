//! Context for Discord family guest access, threaded through tool construction.

/// Threaded into tool construction (see `builder/tools.rs`) when the calling
/// turn is a Discord family guest, so a tool can apply guest-specific gates.
#[derive(Debug, Clone)]
pub struct FamilyGuestContext {
    pub name: String,
    pub origin_channel_id: u64,
    pub owner_name: String,
    pub allowed_harnesses: Vec<String>,
}

impl FamilyGuestContext {
    pub fn from_identity(identity: &hq_core::identity::RequestIdentity) -> Option<Self> {
        let info = identity.family_guest.as_ref()?;
        Some(Self {
            name: info.name.clone(),
            origin_channel_id: info.origin_channel_id,
            owner_name: info.owner_name.clone(),
            allowed_harnesses: info.allowed_harnesses.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::identity::{FamilyGuestInfo, RequestIdentity};

    #[test]
    fn from_identity_none_when_no_family_guest() {
        let id = RequestIdentity::from_discord(123);
        assert!(FamilyGuestContext::from_identity(&id).is_none());
    }

    #[test]
    fn from_identity_extracts_info() {
        let mut id = RequestIdentity::from_discord(123);
        id.family_guest = Some(FamilyGuestInfo {
            name: "Carol".into(),
            origin_channel_id: 123,
            owner_name: "Owner".into(),
            allowed_harnesses: Vec::new(),
        });
        let ctx = FamilyGuestContext::from_identity(&id).unwrap();
        assert_eq!(ctx.name, "Carol");
        assert_eq!(ctx.origin_channel_id, 123);
    }
}
