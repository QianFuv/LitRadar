-- name: Users :many
SELECT u.id,u.username,u.email,u.auth_kind,EXISTS(SELECT 1 FROM user_identities WHERE user_id=u.id),u.is_admin,u.created_at,u.updated_at,(SELECT COUNT(*) FROM folders f WHERE f.user_id=u.id),(SELECT COUNT(*) FROM favorites fv WHERE fv.user_id=u.id),(SELECT COUNT(*) FROM notification_settings ns WHERE ns.user_id=u.id AND ns.enabled=1) FROM users u ORDER BY u.id;

-- name: Invites :many
SELECT ic.id,ic.code,ic.created_by,ic.used_by,ic.used_at,ic.created_at,uc.username,uu.username,ic.expires_at,ic.revoked_at,ic.max_uses,ic.use_count FROM invite_codes ic LEFT JOIN users uc ON ic.created_by=uc.id LEFT JOIN users uu ON ic.used_by=uu.id ORDER BY ic.created_at DESC;

-- name: CreateAdministratorInvite :execresult
INSERT INTO invite_codes(code,created_by,created_at,expires_at,max_uses,use_count) VALUES(?,NULL,?,?,?,0);

-- name: RevokeAdministratorInvite :execresult
UPDATE invite_codes SET revoked_at=MAX(?,created_at) WHERE id=? AND revoked_at IS NULL;

-- name: LastInsertID :one
SELECT last_insert_rowid();

-- name: AuditEvents :many
SELECT id,actor_id,target_id,action,outcome,reason,request_id,source_class,bucket,rejected_count,retry_after_seconds,occurred_at FROM security_audit_events ORDER BY id;

-- name: AuditRetention :one
SELECT last_retention_at FROM security_audit_maintenance WHERE id=1;

-- name: PruneAuditEvents :execresult
DELETE FROM security_audit_events WHERE id IN (SELECT expired.id FROM security_audit_events AS expired WHERE expired.occurred_at<? ORDER BY expired.occurred_at,expired.id LIMIT 10000);

-- name: HasExpiredAuditEvents :one
SELECT EXISTS(SELECT 1 FROM security_audit_events WHERE occurred_at<?);

-- name: RecordAuditRetention :execresult
UPDATE security_audit_maintenance SET last_retention_at=? WHERE id=1;

-- name: CNKISession :one
SELECT session_json,qr_uuid,status,updated_at,last_used_at,generation FROM cnki_sessions WHERE user_id=? AND status<>'empty';

-- name: ReserveCNKISession :one
INSERT INTO cnki_sessions(user_id,session_json,qr_uuid,status,token_expires_at,created_at,updated_at,last_used_at,generation) VALUES(?1,?2,'','empty',NULL,?3,?3,NULL,1) ON CONFLICT(user_id) DO UPDATE SET qr_uuid='',generation=cnki_sessions.generation+1 RETURNING generation;

-- name: CompleteCNKISession :execresult
UPDATE cnki_sessions SET session_json=?1,qr_uuid=?2,status=?3,token_expires_at=?4,updated_at=?5,generation=generation+1 WHERE user_id=?6 AND generation=?7 AND (?8 IS NULL OR qr_uuid=?8);

-- name: StoreCNKISession :one
INSERT INTO cnki_sessions(user_id,session_json,qr_uuid,status,token_expires_at,created_at,updated_at,last_used_at,generation) VALUES(?1,?2,?3,?4,?5,?6,?6,NULL,1) ON CONFLICT(user_id) DO UPDATE SET session_json=excluded.session_json,qr_uuid=excluded.qr_uuid,status=excluded.status,token_expires_at=excluded.token_expires_at,updated_at=excluded.updated_at,generation=cnki_sessions.generation+1 RETURNING generation;

-- name: HasCNKISession :one
SELECT status<>'empty' FROM cnki_sessions WHERE user_id=?;

-- name: ClearCNKISession :execresult
INSERT INTO cnki_sessions(user_id,session_json,qr_uuid,status,token_expires_at,created_at,updated_at,last_used_at,generation) VALUES(?1,?2,'','empty',NULL,?3,?3,NULL,1) ON CONFLICT(user_id) DO UPDATE SET session_json=excluded.session_json,qr_uuid='',status='empty',token_expires_at=NULL,updated_at=excluded.updated_at,last_used_at=NULL,generation=cnki_sessions.generation+1;

-- name: TouchCNKISession :execresult
UPDATE cnki_sessions SET last_used_at=? WHERE user_id=? AND status<>'empty';

-- name: UpdateEmail :execresult
UPDATE users SET email=?,updated_at=? WHERE id=?;

-- name: CredentialsByID :one
SELECT users.id,users.username,users.password_hash,users.salt,users.is_admin,users.created_at,users.token_generation,users.email,users.auth_kind,EXISTS(SELECT 1 FROM user_identities WHERE user_id=users.id) FROM users WHERE users.id=?;

-- name: ObservedSession :one
SELECT EXISTS(SELECT 1 FROM users u JOIN access_tokens t ON t.user_id=u.id WHERE u.id=? AND u.token_generation=? AND u.auth_kind='local' AND t.token_hash=? AND t.expires_at>?);

-- name: IdentityOwner :one
SELECT user_id FROM user_identities WHERE issuer=? AND subject=?;

-- name: HasIdentity :one
SELECT EXISTS(SELECT 1 FROM user_identities WHERE user_id=?);

-- name: CreateIdentity :execresult
INSERT INTO user_identities(user_id,issuer,subject,created_at,last_login_at) VALUES(?,?,?,?,?);

-- name: HasEmail :one
SELECT EXISTS(SELECT 1 FROM users WHERE email=?);

-- name: SetOIDCEmail :execresult
UPDATE users SET email=?,auth_kind='oidc' WHERE id=?;

-- name: HasOtherEmail :one
SELECT EXISTS(SELECT 1 FROM users WHERE email=? AND id!=?);

-- name: RecordIdentityLogin :execresult
UPDATE user_identities SET last_login_at=? WHERE issuer=? AND subject=?;

-- name: DeleteIdentity :execresult
DELETE FROM user_identities WHERE user_id=?;

-- name: ActiveInviteID :one
SELECT id FROM invite_codes WHERE created_by=? AND revoked_at IS NULL;

-- name: RevokeOwnedInvites :execresult
UPDATE invite_codes SET revoked_at=MAX(?,created_at) WHERE created_by=? AND revoked_at IS NULL;

-- name: CreateInvite :execresult
INSERT INTO invite_codes(code,created_by,created_at,expires_at,max_uses,use_count) VALUES(?,?,?,?,?,0);

-- name: RevokeInvite :one
UPDATE invite_codes SET revoked_at=MAX(?,created_at) WHERE created_by=? AND revoked_at IS NULL RETURNING id;

-- name: LatestInvite :one
SELECT id,code,used_by,used_at,expires_at,revoked_at,max_uses,use_count,created_at FROM invite_codes WHERE created_by=? ORDER BY created_at DESC,id DESC LIMIT 1;

-- name: InsertAudit :execresult
INSERT INTO security_audit_events(actor_id,target_id,action,outcome,reason,request_id,source_class,bucket,rejected_count,retry_after_seconds,occurred_at) VALUES(?,?,?,?,?,?,?,?,?,?,?);

-- name: Administrator :one
SELECT is_admin FROM users WHERE users.id=?;

-- name: UserCount :one
SELECT count(*) FROM users;

-- name: RedeemInvite :one
UPDATE invite_codes SET used_by=CASE WHEN use_count=0 THEN ?1 ELSE used_by END,used_at=CASE WHEN use_count=0 THEN ?2 ELSE used_at END,use_count=use_count+1 WHERE code=?3 AND revoked_at IS NULL AND expires_at>?2 AND use_count<max_uses RETURNING id;

-- name: RecordInviteUse :execresult
INSERT INTO invite_code_uses(invite_code_id,user_id,used_at) VALUES(?,?,?);

-- name: CreateUser :execresult
INSERT INTO users(username,password_hash,salt,is_admin,created_at,updated_at) VALUES(?,?,?,?,?,?);

-- name: CreateDefaultFolder :execresult
INSERT INTO folders(user_id,name,is_tracking,created_at,updated_at) VALUES(?,?,1,?,?);

-- name: CredentialsByEmail :one
SELECT users.id,users.username,users.password_hash,users.salt,users.is_admin,users.created_at,users.token_generation,users.email,users.auth_kind,EXISTS(SELECT 1 FROM user_identities WHERE user_id=users.id) FROM users WHERE email=?;

-- name: CredentialsByName :one
SELECT users.id,users.username,users.password_hash,users.salt,users.is_admin,users.created_at,users.token_generation,users.email,users.auth_kind,EXISTS(SELECT 1 FROM user_identities WHERE user_id=users.id) FROM users WHERE username=?;

-- name: HasUsers :one
SELECT EXISTS(SELECT 1 FROM users);

-- name: SetAdministrator :execresult
UPDATE users SET is_admin=?,updated_at=? WHERE id=?;

-- name: DeleteUser :execresult
DELETE FROM users WHERE users.id=?;

-- name: AdministratorCount :one
SELECT count(*) FROM users WHERE is_admin=1;

-- name: TokenGeneration :one
SELECT token_generation FROM users WHERE users.id=?;

-- name: HasActiveToken :one
SELECT EXISTS(SELECT 1 FROM access_tokens WHERE user_id=? AND token_hash=? AND expires_at>?);

-- name: PruneTokens :execresult
DELETE FROM access_tokens WHERE expires_at<=?;

-- name: DeleteLoginToken :execresult
DELETE FROM access_tokens WHERE user_id=? AND name='login';

-- name: TokenCount :one
SELECT count(*) FROM access_tokens WHERE user_id=? AND expires_at>? AND name!='login';

-- name: CreateToken :execresult
INSERT INTO access_tokens(user_id,token_hash,name,expires_at,created_at) VALUES(?,?,?,?,?);

-- name: TokenAuthorization :one
SELECT t.user_id,t.expires_at,u.username,u.is_admin,u.created_at,u.token_generation,u.email,u.auth_kind,EXISTS(SELECT 1 FROM user_identities WHERE user_id=u.id) FROM access_tokens t JOIN users u ON t.user_id=u.id WHERE t.token_hash=?;

-- name: DeleteTokenHash :execresult
DELETE FROM access_tokens WHERE token_hash=?;

-- name: Tokens :many
SELECT id,name,expires_at,created_at FROM access_tokens WHERE user_id=? AND expires_at>? AND name!='login' ORDER BY created_at DESC;

-- name: DeleteTokenID :execresult
DELETE FROM access_tokens WHERE id=? AND user_id=?;

-- name: TokenID :one
SELECT id FROM access_tokens WHERE token_hash=?;

-- name: AdvanceTokenGeneration :execresult
UPDATE users SET token_generation=token_generation+1 WHERE id=?;

-- name: DeleteUserTokens :execresult
DELETE FROM access_tokens WHERE user_id=?;

-- name: UpgradePassword :execresult
UPDATE users SET password_hash=?,salt='',updated_at=? WHERE id=? AND password_hash=? AND salt=?;

-- name: ChangePassword :execresult
UPDATE users SET password_hash=?,salt=?,updated_at=?,token_generation=token_generation+1 WHERE id=? AND password_hash=? AND salt=? AND auth_kind='local';

-- name: AuthKind :one
SELECT auth_kind FROM users WHERE users.id=?;

-- name: ResetPassword :execresult
UPDATE users SET password_hash=?,salt=?,updated_at=?,token_generation=token_generation+1 WHERE id=?;

