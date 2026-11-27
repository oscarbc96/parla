-- The sidebar names a group after its participants: a member's short user
-- name, and a group's participants (newline separated user names).
ALTER TABLE members ADD COLUMN handle TEXT;
ALTER TABLE conversations ADD COLUMN participants TEXT;
