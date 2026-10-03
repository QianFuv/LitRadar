package zjlib

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
	"github.com/QianFuv/LitRadar/internal/transport"
)

const searchDatabaseValue = "中国学术期刊网络出版总库,中国博士学位论文全文数据库,中国优秀硕士学位论文全文数据库,中国重要会议论文全文数据库,中国重要报纸全文数据库,中国年鉴网络出版总库"

func searchResultFields(keyword string) []scholarly.QueryPair {
	return []scholarly.QueryPair{
		pair("dbPrefix", "SCDB"), pair("db_opt", "中国学术文献网络出版总库"), pair("db_value", searchDatabaseValue), pair("hidTabChange", ""), pair("hidDivIDS", ""), pair("txt_i", "1"), pair("txt_c", "7"), pair("{key}_logical", "and"), pair("txt_1_sel", "主题"), pair("txt_1_value1", keyword), pair("txt_1_freq1", ""), pair("txt_1_relation", "#CNKI_AND"), pair("txt_1_value2", "输入检索词"), pair("txt_1_freq2", ""), pair("txt_1_special1", "="), pair("txt_extension", "xls"), pair("tmpexpertvalue", ""), pair("expertValue", ""), pair("cjfdcode", ""), pair("currentid", "txt_1_value1"), pair("action", "scdbsearch"),
	}
}
func searchHandlerFields(keyword string, millis int64) []scholarly.QueryPair {
	return []scholarly.QueryPair{
		pair("action", ""), pair("NaviCode", "*"), pair("PageName", "ASP.brief_result_aspx"), pair("DbPrefix", "SCDB"), pair("DbCatalog", "中国学术文献网络出版总库"), pair("ConfigFile", "SCDB.xml"), pair("db_opt", "中国学术文献网络出版总库"), pair("db_value", searchDatabaseValue), pair("txt_1_sel", "主题"), pair("txt_1_value1", keyword), pair("txt_1_relation", "#CNKI_AND"), pair("txt_1_special1", "="), pair("txt_1_extension", "xls"), pair("his", "0"), pair("__", strconv.FormatInt(millis, 10)),
	}
}
func (live *LiveTransport) postFormText(ctx context.Context, location string, fields []scholarly.QueryPair, headers http.Header, action string) (string, error) {
	if _, err := live.state.allowed.parse(location, proxyFamily); err != nil {
		return "", err
	}
	response, finish, err := live.send(ctx, http.MethodPost, location, scholarly.EncodeQuery(fields), headers, true)
	if err != nil {
		return "", err
	}
	defer finish()
	if err = raiseForStatus(response, action); err != nil {
		return "", err
	}
	return responseText(response)
}

// Search executes the three-request legacy CNKI search protocol before applying the result limit.
func (live *LiveTransport) Search(ctx context.Context, keyword string, limit uint64) ([]SearchResult, error) {
	if err := live.enter(ctx); err != nil {
		return nil, err
	}
	defer live.leave()
	base := live.state.allowed.bases[proxyFamily]
	resultUrl := appendEndpoint(base, "/kns55/brief/result.aspx")
	handlerUrl := appendEndpoint(base, "/kns55/request/SearchHandler.ashx")
	briefUrl := appendEndpoint(base, "/kns55/brief/brief.aspx")
	headers := formHeaders(appendEndpoint(base, "/kns55/"), origin(base))
	if _, err := live.postFormText(ctx, resultUrl, searchResultFields(keyword), headers, "post CNKI result.aspx"); err != nil {
		return nil, err
	}
	headers.Set("Referer", resultUrl)
	headers.Set("X-Requested-With", "XMLHttpRequest")
	if _, err := live.postFormText(ctx, handlerUrl, searchHandlerFields(keyword, time.Now().UnixMilli()), headers, "post CNKI SearchHandler"); err != nil {
		return nil, err
	}
	location := briefUrl + "?" + encodePairs(pair("pagename", "ASP.brief_result_aspx"), pair("dbPrefix", "SCDB"), pair("dbCatalog", "中国学术文献网络出版总库"), pair("ConfigFile", "SCDB.xml"), pair("research", "off"), pair("t", strconv.FormatInt(time.Now().UnixMilli(), 10)))
	response, finish, err := live.send(ctx, http.MethodGet, location, "", htmlHeaders(resultUrl), true)
	if err != nil {
		return nil, err
	}
	defer finish()
	if err = raiseForStatus(response, "get CNKI brief results"); err != nil {
		return nil, err
	}
	final := response.Request.URL.String()
	text, err := responseText(response)
	if err != nil {
		return nil, err
	}
	live.state.lastBriefUrl = &final
	results, err := parseSearchResults(text, final, live.state.allowed)
	if err != nil {
		return nil, err
	}
	if uint64(len(results)) > limit {
		results = results[:limit]
	}
	return results, nil
}

// InspectResultMetadata fetches the canonical detail page and ignores unverified row-level PDF hints.
func (live *LiveTransport) InspectResultMetadata(ctx context.Context, result SearchResult) (ArticleCandidate, error) {
	if err := live.enter(ctx); err != nil {
		return ArticleCandidate{}, err
	}
	defer live.leave()
	referer := appendEndpoint(live.state.allowed.bases[proxyFamily], "/kns55/")
	if live.state.lastBriefUrl != nil {
		referer = *live.state.lastBriefUrl
	}
	location, err := live.state.allowed.parse(result.DetailUrl, proxyFamily)
	if err != nil {
		return ArticleCandidate{}, err
	}
	response, finish, err := live.send(ctx, http.MethodGet, location.Href(false), "", htmlHeaders(referer), true)
	if err != nil {
		return ArticleCandidate{}, err
	}
	defer finish()
	if err = raiseForStatus(response, "open CNKI detail"); err != nil {
		return ArticleCandidate{}, err
	}
	final := response.Request.URL.String()
	text, err := responseText(response)
	if err != nil {
		return ArticleCandidate{}, err
	}
	identity := ExtractArticleIdentity(text, result.Title)
	pdf, err := extractDownloadUrl(text, final, live.state.allowed, true)
	if err != nil {
		return ArticleCandidate{}, err
	}
	return ArticleCandidate{Result: cloneResult(result), Identity: identity, DetailUrl: final, PdfUrl: pdf}, nil
}

// DownloadPdf enforces the decoded size limit and retains content-type-or-magic PDF recognition.
func (live *LiveTransport) DownloadPdf(ctx context.Context, pdfUrl string, title, referer *string) (DownloadedPdf, error) {
	if err := live.enter(ctx); err != nil {
		return DownloadedPdf{}, err
	}
	defer live.leave()
	location, err := live.state.allowed.parse(pdfUrl, proxyFamily)
	if err != nil {
		return DownloadedPdf{}, err
	}
	source := appendEndpoint(live.state.allowed.bases[proxyFamily], "/kns55/")
	if referer != nil {
		source = *referer
	}
	response, finish, err := live.send(ctx, http.MethodGet, location.Href(false), "", htmlHeaders(source), true)
	if err != nil {
		return DownloadedPdf{}, err
	}
	defer finish()
	if err = raiseForStatus(response, "download PDF"); err != nil {
		return DownloadedPdf{}, err
	}
	final := response.Request.URL.String()
	contentType := "application/pdf"
	if values, ok := response.Header["Content-Type"]; ok && len(values) > 0 && asciiHeader(values[0]) {
		contentType = values[0]
	}
	content, err := transport.BoundedBytes(response, live.state.config.MaximumDocumentBytes)
	if err != nil {
		if errors.Is(err, transport.ErrTooLarge) {
			return DownloadedPdf{}, &Error{Kind: "Request", Message: "Download endpoint exceeded the configured document size limit."}
		}
		return DownloadedPdf{}, &Error{Kind: "Request", Message: "request or response body error"}
	}
	if !strings.Contains(asciiLower(contentType), "pdf") && !bytes.HasPrefix(content, []byte("%PDF")) {
		return DownloadedPdf{}, &Error{Kind: "Request", Message: fmt.Sprintf("Download endpoint did not return PDF (content-type=%q, url=%s).", contentType, redactUrl(final))}
	}
	resolved := title
	if resolved == nil || strings.TrimSpace(*resolved) == "" {
		resolved = titleFromPdfUrl(final)
	}
	if resolved == nil {
		resolved = pointer("cnki")
	}
	return DownloadedPdf{Filename: SafeFilename(*resolved) + ".pdf", FinalUrl: final, ContentType: contentType, ByteCount: uint64(len(content)), Content: content}, nil
}

var _ Transport = (*LiveTransport)(nil)
