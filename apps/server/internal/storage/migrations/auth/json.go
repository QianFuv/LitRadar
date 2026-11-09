package auth

import "github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"

func validJson(value string) bool { return jsonvalue.ValidJson(value) }
