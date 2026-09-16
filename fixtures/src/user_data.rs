struct UserData {
    name: String,
    email: String,
}

impl UserData {
    fn new(name: &str, email: &str) -> Self {
        UserData {
            name: name.to_string(),
            email: email.to_string(),
        }
    }

    fn anonymize(&self) -> String {
        format!("{}@{}", self.name, self.email.split('@').nth(1).unwrap_or("unknown"))
    }
}
