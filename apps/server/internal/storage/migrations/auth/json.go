package auth

import "github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"

func validJSON(value string) bool { return jsonvalue.ValidJSON(value) }
