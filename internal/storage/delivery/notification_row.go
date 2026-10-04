package delivery

import (
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

type notificationRow struct {
	values []any
	err    error
}

func readNotificationRow(row rowScanner, count int) *notificationRow {
	result := &notificationRow{values: make([]any, count)}
	targets := make([]any, count)
	for index := range result.values {
		targets[index] = &result.values[index]
	}
	result.err = row.Scan(targets...)
	return result
}

func (row *notificationRow) text(index int) string {
	if row.err != nil {
		return ""
	}
	var value sqlite.Text
	row.err = value.Scan(row.values[index])
	return string(value)
}

func (row *notificationRow) integer(index int) int64 {
	if row.err != nil {
		return 0
	}
	var value sqlite.Integer
	row.err = value.Scan(row.values[index])
	return int64(value)
}

func (row *notificationRow) number(index int) float64 {
	if row.err != nil {
		return 0
	}
	var value sqlite.Number
	row.err = value.Scan(row.values[index])
	return float64(value)
}

func (row *notificationRow) optionalInteger(index int) *int64 {
	if row.err != nil {
		return nil
	}
	var value sqlite.OptionalInteger
	row.err = value.Scan(row.values[index])
	return value.Value
}

func (row *notificationRow) strings(index int) []string {
	value := row.text(index)
	if row.err != nil {
		return nil
	}
	var values []string
	values, row.err = notificationStrings(value)
	return values
}

func (row *notificationRow) secret(index int, codec *secrets.Codec, userId int64, field string) string {
	value := row.text(index)
	if row.err != nil {
		return ""
	}
	var decoded string
	decoded, row.err = codec.Decrypt(value, secrets.NotificationContext(userId, field))
	return decoded
}
