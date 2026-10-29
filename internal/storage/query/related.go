package query

import (
	"context"
	"database/sql"
	"sort"
	"strings"
	"sync"
	"unicode"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/search"
)

// RelatedArticleMaxLimit bounds related-article responses.
const RelatedArticleMaxLimit int64 = 30

const relatedTermLimit = 16

// relatedCandidateFactor over-fetches ranked candidates so duplicate titles can be dropped.
const relatedCandidateFactor = 3

// relatedRecallBudget bounds the bm25 work of one recall. Ranking cost grows with the number of
// matches, so recall uses the rarest terms whose combined costs fit this budget.
const relatedRecallBudget = 30000

// relatedHanCostFactor weights Han phrases, which the per-character index matches by comparing
// character positions and which therefore cost several times more per match than Latin words.
const relatedHanCostFactor = 4

// relatedCountCacheLimit bounds the per-process document count cache.
const relatedCountCacheLimit = 100000

// relatedCounts caches term recall costs by database file and term. Costs only steer term
// selection, so slightly stale values after an index update are harmless.
var relatedCounts = struct {
	sync.Mutex
	values map[string]int
}{values: map[string]int{}}

// relatedRanking weights the FTS5 columns (article_id, title, abstract_text, doi, pmid, authors,
// journal_title) so title-to-title overlap dominates and abstract overlap mainly breaks ties.
const relatedRanking = "bm25(article_search,0,10,1,0,0,1,0)"

// relatedLatinStopwords holds function words that occur in most titles. Matching them forces
// bm25 to score a large share of the corpus while contributing almost no relevance.
var relatedLatinStopwords = map[string]bool{}

// relatedHanStopRunes marks particles that make a Han bigram meaningless.
var relatedHanStopRunes = map[rune]bool{}

// relatedHanStopBigrams lists generic academic bigrams shared by a large share of Chinese titles.
var relatedHanStopBigrams = map[string]bool{}

func init() {
	for _, word := range strings.Fields("a about after against among an and are as at based be between by can does during for from has have how in into is it its of on or over than that the their these this through to toward towards under using via was what when where which while with within without") {
		relatedLatinStopwords[word] = true
	}
	for _, character := range "的与和及对在中为之于或等是了把被从以其" {
		relatedHanStopRunes[character] = true
	}
	for _, bigram := range strings.Fields("研究 分析 基于 影响 视角 探讨 问题 发展 实证 理论 机制 路径 效应 对策 现状 思考 我国 中国") {
		relatedHanStopBigrams[bigram] = true
	}
}

// relatedTerms tokenizes a title into distinct match terms: Latin words except function words,
// short Han runs as phrases, and longer Han runs as overlapping bigrams without particles or
// generic academic bigrams. At most sixteen terms are kept in title order.
func relatedTerms(title string) []string {
	terms := []string{}
	seen := map[string]bool{}
	add := func(term string) bool {
		if term == "" || seen[term] {
			return len(terms) < relatedTermLimit
		}
		seen[term] = true
		terms = append(terms, term)
		return len(terms) < relatedTermLimit
	}
	runes := []rune(strings.ToLower(title))
	for index := 0; index < len(runes); {
		character := runes[index]
		switch {
		case unicode.Is(unicode.Han, character):
			end := index
			for end < len(runes) && unicode.Is(unicode.Han, runes[end]) {
				end++
			}
			run := runes[index:end]
			index = end
			if len(run) < 2 {
				continue
			}
			if len(run) <= 3 {
				if !add(string(run)) {
					return terms
				}
				continue
			}
			for start := 0; start+2 <= len(run); start++ {
				bigram := run[start : start+2]
				if relatedHanStopRunes[bigram[0]] || relatedHanStopRunes[bigram[1]] || relatedHanStopBigrams[string(bigram)] {
					continue
				}
				if !add(string(bigram)) {
					return terms
				}
			}
		case unicode.IsLetter(character) || unicode.IsNumber(character):
			end := index
			for end < len(runes) && (unicode.IsLetter(runes[end]) || unicode.IsNumber(runes[end])) && !unicode.Is(unicode.Han, runes[end]) {
				end++
			}
			word := string(runes[index:end])
			index = end
			if len([]rune(word)) < 2 || relatedLatinStopwords[word] {
				continue
			}
			if !add(word) {
				return terms
			}
		default:
			index++
		}
	}
	return terms
}

// relatedMatch builds an FTS5 expression matching any term in titles or abstracts.
func relatedMatch(terms []string) string {
	return "{title abstract_text} : (" + relatedAny(terms) + ")"
}

// relatedAny joins quoted FTS5 phrases with OR.
func relatedAny(terms []string) string {
	quoted := make([]string, len(terms))
	for index, term := range terms {
		quoted[index] = `"` + strings.ReplaceAll(term, `"`, `""`) + `"`
	}
	return strings.Join(quoted, " OR ")
}

// relatedExpression recalls with the cheapest terms whose combined costs fit the recall budget.
// When even the cheapest term is too common, it requires that term together with any other.
func relatedExpression(terms []string, costs []int) string {
	order := make([]int, len(terms))
	for index := range order {
		order[index] = index
	}
	sort.SliceStable(order, func(left, right int) bool { return costs[order[left]] < costs[order[right]] })
	selected, total := []string{}, 0
	for _, index := range order {
		if total+costs[index] > relatedRecallBudget {
			break
		}
		total += costs[index]
		selected = append(selected, terms[index])
	}
	if len(selected) > 0 {
		ranking := []string{}
		for _, index := range order[len(selected):] {
			if !relatedIsHan(terms[index]) {
				ranking = append(ranking, terms[index])
			}
		}
		if len(ranking) == 0 {
			return relatedMatch(selected)
		}
		return "{title abstract_text} : ((" + relatedAny(selected) + ") AND (" + relatedAny(append(selected, ranking...)) + "))"
	}
	rarest, others := terms[order[0]], []string{}
	for _, index := range order[1:] {
		others = append(others, terms[index])
	}
	if len(others) == 0 {
		return relatedMatch([]string{rarest})
	}
	return "{title abstract_text} : (" + relatedAny([]string{rarest}) + " AND (" + relatedAny(others) + "))"
}

// relatedIsHan reports whether a term is a Han phrase.
func relatedIsHan(term string) bool {
	return strings.IndexFunc(term, func(character rune) bool { return unicode.Is(unicode.Han, character) }) >= 0
}

// relatedTermCost returns the recall cost of a term: its document count in titles and abstracts,
// weighted for Han phrases. Counting stops just past the budget, and results are cached.
func relatedTermCost(ctx context.Context, connection *sql.Conn, filename string, term string) (int, error) {
	key := filename + "\x00" + term
	relatedCounts.Lock()
	cost, found := relatedCounts.values[key]
	relatedCounts.Unlock()
	if found {
		return cost, nil
	}
	weight := 1
	if relatedIsHan(term) {
		weight = relatedHanCostFactor
	}
	var count int
	if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM (SELECT 1 FROM article_search WHERE article_search MATCH ? LIMIT ?)", relatedMatch([]string{term}), relatedRecallBudget/weight+1).Scan(&count); err != nil {
		return 0, err
	}
	cost = count * weight
	relatedCounts.Lock()
	if len(relatedCounts.values) >= relatedCountCacheLimit {
		relatedCounts.values = map[string]int{}
	}
	relatedCounts.values[key] = cost
	relatedCounts.Unlock()
	return cost, nil
}

// RelatedArticles returns articles ranked by FTS5 bm25 similarity to the source article title.
func RelatedArticles(ctx context.Context, configuration config.Config, name *string, id int64, limit int64) ([]domain.Article, error) {
	if limit < 1 || limit > RelatedArticleMaxLimit {
		return nil, InvalidInput{"limit must be between 1 and 30"}
	}
	filename, err := configuration.ResolveIndexDbPath(name)
	if err != nil {
		return nil, err
	}
	database, err := open(configuration, name)
	if err != nil {
		return nil, err
	}
	defer database.Close()
	sources, err := fetchArticles(ctx, database, []int64{id})
	if err != nil {
		return nil, err
	}
	if len(sources) == 0 {
		return nil, NotFound{"Article not found"}
	}
	ids, err := relatedArticleIds(ctx, database, filename, sources[0].Title, id, limit*relatedCandidateFactor)
	if err != nil {
		return nil, err
	}
	candidates, err := fetchArticles(ctx, database, ids)
	if err != nil {
		return nil, err
	}
	return distinctRelatedArticles(sources[0].Title, candidates, limit), nil
}

// distinctRelatedArticles keeps ranked candidates whose titles differ from the source and from
// each other, so reprinted notices and duplicate records do not fill the list.
func distinctRelatedArticles(sourceTitle string, candidates []domain.Article, limit int64) []domain.Article {
	seen := map[string]bool{relatedTitleKey(sourceTitle): true}
	related := []domain.Article{}
	for _, candidate := range candidates {
		key := relatedTitleKey(candidate.Title)
		if seen[key] {
			continue
		}
		seen[key] = true
		related = append(related, candidate)
		if int64(len(related)) == limit {
			break
		}
	}
	return related
}

// relatedTitleKey compares titles by their letters and digits only.
func relatedTitleKey(title string) string {
	return strings.Map(func(character rune) rune {
		if unicode.IsLetter(character) || unicode.IsNumber(character) {
			return unicode.ToLower(character)
		}
		return -1
	}, title)
}

// relatedArticleIds ranks matching rowids on a connection that is released before enrichment,
// because index databases are opened with a single connection.
func relatedArticleIds(ctx context.Context, database *sql.DB, filename string, title string, id int64, limit int64) ([]int64, error) {
	connection, err := database.Conn(ctx)
	if err != nil {
		return nil, err
	}
	defer connection.Close()
	usesSimple, err := search.UsesSimple(ctx, connection)
	if err != nil {
		return nil, err
	}
	terms := relatedTerms(search.PrepareText(title, usesSimple))
	if len(terms) == 0 {
		return []int64{}, nil
	}
	costs := make([]int, len(terms))
	for index, term := range terms {
		if costs[index], err = relatedTermCost(ctx, connection, filename, term); err != nil {
			return nil, err
		}
	}
	rows, err := connection.QueryContext(ctx, "SELECT rowid FROM article_search WHERE article_search MATCH ? AND rowid<>? ORDER BY "+relatedRanking+" LIMIT ?", relatedExpression(terms, costs), id, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	ids := []int64{}
	for rows.Next() {
		var related int64
		if err := rows.Scan(&related); err != nil {
			return nil, err
		}
		ids = append(ids, related)
	}
	return ids, rows.Err()
}
