package auth

import "github.com/QianFuv/LitRadar/internal/compat/jsonvalue"

func validJson(value string) bool { return jsonvalue.ValidJson(value) }
