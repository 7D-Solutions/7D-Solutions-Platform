-- RBAC data model: roles, permissions, bindings
-- All tenant-scoped where appropriate

-- Permissions are global stable strings (e.g. "gl.journal.create")
CREATE TABLE permissions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key VARCHAR(128) NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_permissions_key ON permissions(key);

-- Roles are tenant-scoped
CREATE TABLE roles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    name VARCHAR(128) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    is_system BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(tenant_id, name)
);

CREATE INDEX idx_roles_tenant ON roles(tenant_id);

-- Junction: which permissions a role grants
CREATE TABLE role_permissions (
    role_id UUID NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    permission_id UUID NOT NULL REFERENCES permissions(id) ON DELETE CASCADE,
    granted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (role_id, permission_id)
);

CREATE INDEX idx_role_permissions_role ON role_permissions(role_id);
CREATE INDEX idx_role_permissions_perm ON role_permissions(permission_id);

-- User-role bindings: which roles a user holds in a tenant
CREATE TABLE user_role_bindings (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL,
    role_id UUID NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    granted_by UUID,
    granted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    UNIQUE(tenant_id, user_id, role_id)
);

CREATE INDEX idx_urb_tenant_user ON user_role_bindings(tenant_id, user_id);
CREATE INDEX idx_urb_role ON user_role_bindings(role_id);
CREATE INDEX idx_urb_active ON user_role_bindings(tenant_id, user_id) WHERE revoked_at IS NULL;
