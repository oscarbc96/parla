-- The channel details panel shows a channel's description (purpose) from the
-- cache before the provider refills it.
ALTER TABLE conversations ADD COLUMN purpose TEXT;
