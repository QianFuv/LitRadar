package cnki

import domain "github.com/QianFuv/LitRadar/internal/domain/sources"

// JournalSearchForm preserves the upstream navigation search protocol and field spelling.
func JournalSearchForm(keyword, fieldName string) map[string]string {
	state := map[string]any{
		"StateID": "", "Platfrom": "", "QueryTime": "", "Account": "knavi", "ClientToken": "", "Language": "",
		"CNode": map[string]any{"PCode": "OYXNO5VW", "SMode": "", "OperateT": ""},
		"QNode": map[string]any{
			"SelectT": "", "Select_Fields": "", "S_DBCodes": "", "Subscribed": "", "OrderBy": "OTA|DESC", "GroupBy": "", "Additon": "",
			"QGroup": []any{map[string]any{"Key": "subject", "Logic": 1, "Items": []any{}, "ChildItems": []any{map[string]any{
				"Key": "txt", "Logic": 1, "ChildItems": []any{}, "Items": []any{map[string]any{
					"Key": "txt_1", "Title": "", "Logic": 1, "Name": fieldName, "Operate": "%", "Value": keyword, "ExtendType": 0, "ExtendValue": "", "Value2": "",
				}},
			}}}},
		},
	}
	encoded, err := domain.Json(state)
	if err != nil {
		encoded = []byte("{}")
	}
	return map[string]string{"searchStateJson": string(encoded), "displaymode": "1", "pageindex": "1", "pagecount": "21", "index": "JSTMWT6S", "searchType": "刊名(曾用刊名)", "parentcode": "SQN63324", "clickName": "", "switchdata": "search"}
}
