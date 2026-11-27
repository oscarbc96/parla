-- The workspace's user groups (replaced as a whole on each fetch, at most
-- domain::MAX_USER_GROUPS rows), for @ completion before the provider answers.
CREATE TABLE user_groups (
    id TEXT PRIMARY KEY,
    handle TEXT NOT NULL,
    name TEXT NOT NULL,
    member_count INTEGER NOT NULL,
    is_member INTEGER NOT NULL
);
