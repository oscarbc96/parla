-- A member's job title and IANA time zone from the directory (null when
-- unknown), for the People view.
ALTER TABLE members ADD COLUMN title TEXT;
ALTER TABLE members ADD COLUMN tz TEXT;
