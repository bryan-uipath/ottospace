// Made-up sign-in check.
pub struct User {
    pub name: String,
    pub admin: bool,
}

pub fn may_sign_in(user: &User, open: bool) -> bool {
    user.admin || open
}
