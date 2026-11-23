use crate::features::identity::AuthUser;

use super::service::{display_name, reject_service_principal};

fn account(auth_source: &str) -> AuthUser {
    AuthUser {
        id: "user-1".into(),
        username: "ci".into(),
        display_name: "CI".into(),
        auth_source: auth_source.into(),
        email: None,
        is_admin: false,
    }
}

#[test]
fn a_service_principal_cannot_manage_service_accounts() {
    assert!(reject_service_principal(&account("service")).is_err());
    assert!(reject_service_principal(&account("local")).is_ok());
    assert!(reject_service_principal(&account("ldap")).is_ok());
}

#[test]
fn the_display_name_is_trimmed_and_required() {
    assert_eq!(display_name("  Deploy bot  ").unwrap_or(""), "Deploy bot");
    assert!(display_name("   ").is_err());
}
