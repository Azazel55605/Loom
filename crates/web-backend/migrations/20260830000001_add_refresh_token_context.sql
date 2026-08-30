-- Session rows need enough context for a person to recognise the device they
-- are deciding whether to revoke. Both values are best-effort: old rows and
-- unusual clients may not have supplied a user agent, while tests or alternate
-- in-process callers may not carry socket connection metadata.
ALTER TABLE refresh_tokens ADD COLUMN user_agent TEXT NULL;
ALTER TABLE refresh_tokens ADD COLUMN ip_address TEXT NULL;
