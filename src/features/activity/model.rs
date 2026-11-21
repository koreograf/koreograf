/// One append-only activity entry. Entries are never updated in place.
#[derive(Clone, Debug)]
pub(crate) struct Activity {
    pub id: String,
    pub team_id: String,
    pub actor: String,
    pub action: String,
    pub object_type: String,
    pub object_id: String,
    pub object_name: String,
    pub result: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ActivityQuery {
    pub limit: i64,
    pub offset: i64,
    pub actor: String,
    pub action: String,
    pub object_type: String,
    pub since: String,
    pub until: String,
}

impl Default for ActivityQuery {
    fn default() -> Self {
        Self {
            limit: 25,
            offset: 0,
            actor: String::new(),
            action: String::new(),
            object_type: String::new(),
            since: String::new(),
            until: String::new(),
        }
    }
}
