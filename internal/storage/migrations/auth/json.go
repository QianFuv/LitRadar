package auth

import domain "github.com/QianFuv/LitRadar/internal/domain/auth"

func validJson(value string) bool { return domain.ValidJson(value) }
