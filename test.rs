use std::collections::HashMap;

struct User {
    name: String,
    email: String,
    age: u32,
}

impl User {
    fn new(name: &str, email: &str, age: u32) -> Self {
        Self {
            name: name.to_string(),
            email: email.to_string(),
            age,
        }
    }

    fn is_adult(&self) -> bool {
        self.age >= 18
    }

    fn display_name(&self) -> &str {
        &self.name
    }
}

struct UserService {
    users: Vec<User>,
    index: HashMap<String, usize>,
}

impl UserService {
    fn new() -> Self {
        Self {
            users: Vec::new(),
            index: HashMap::new(),
        }
    }

    fn add_user(&mut self, user: User) {
        let idx = self.users.len();
        self.index.insert(user.email.clone(), idx);
        self.users.push(user);
    }

    fn find_by_email(&self, email: &str) -> Option<&User> {
        self.index.get(email).and_then(|&idx| self.users.get(idx))
    }

    fn adults(&self) -> Vec<&User> {
        self.users.iter().filter(|u| u.is_adult()).collect()
    }
}

fn main() {
    let mut service = UserService::new();

    service.add_user(User::new("Alice", "alice@example.com", 25));
    service.add_user(User::new("Bob", "bob@example.com", 17));
    service.add_user(User::new("Charlie", "charlie@example.com", 30));

    println!("Adults:");
    for user in service.adults() {
        println!("  {} ({})", user.display_name(), user.email);
    }
}
