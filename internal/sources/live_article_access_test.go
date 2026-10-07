package sources

import (
	"context"
	"encoding/json"
	"errors"
	"reflect"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
	"github.com/QianFuv/LitRadar/internal/sources/zjlib"
	authstorage "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/transport"
)

type accessSessionReader struct {
	data   *authstorage.CnkiData
	err    error
	calls  int
	user   identity.Id
	active bool
}

func (reader *accessSessionReader) Data(_ context.Context, user identity.Id, active bool) (*authstorage.CnkiData, error) {
	reader.calls++
	reader.user = user
	reader.active = active
	return reader.data, reader.err
}

type observedZjlibTransport struct {
	*zjlib.FixtureTransport
	limit uint64
}

func (wire *observedZjlibTransport) Search(ctx context.Context, title string, limit uint64) ([]zjlib.SearchResult, error) {
	wire.limit = limit
	return wire.FixtureTransport.Search(ctx, title, limit)
}

func TestZjlibFullTextAuthenticationPrecedesTransport(t *testing.T) {
	for _, test := range []struct {
		user    *identity.Id
		data    *authstorage.CnkiData
		err     error
		kind    provider.ErrorKind
		message string
	}{
		{nil, nil, nil, provider.AuthenticationRequired, "authenticated CNKI session required"},
		{new(identity.Id(9007199254740993)), nil, errors.New("private database path"), provider.Internal, "CNKI session unavailable"},
		{new(identity.Id(3)), nil, nil, provider.AuthenticationRequired, "active CNKI session required"},
	} {
		t.Run(test.message, func(t *testing.T) {
			reader := &accessSessionReader{data: test.data, err: test.err}
			access := NewZjlibFullTextProvider(reader, transport.Proxy{})
			access.create = func(time.Time) (zjlib.Transport, func(), error) {
				t.Fatal("unauthenticated request created transport")
				return nil, nil, nil
			}
			_, err := access.ResolveFullText(context.Background(), domain.ArticleLocator{}, domain.ArticleAccessContext{UserId: test.user})
			var failure *provider.Error
			if !errors.As(err, &failure) || failure.Kind != test.kind || failure.Message != test.message {
				t.Fatal(err)
			}
			if test.user == nil {
				if reader.calls != 0 {
					t.Fatal("anonymous session lookup")
				}
			} else if reader.calls != 1 || reader.user != *test.user || !reader.active {
				t.Fatal("wrong active session lookup", reader)
			}
		})
	}
}

// TestZjlibFullTextUsesFreshSessionAndExactTenCandidateMatch verifies exact documents and caller-owned session lifecycle.
func TestZjlibFullTextUsesFreshSessionAndExactTenCandidateMatch(t *testing.T) {
	reader := &accessSessionReader{data: &authstorage.CnkiData{SessionData: json.RawMessage(`{"bff_user_token":"fixture-token","cookies":[]}`)}}
	before := string(reader.data.SessionData)
	access := NewZjlibFullTextProvider(reader, transport.Proxy{})
	deadline := time.Now().Add(time.Minute)
	created, closed := 0, 0
	wires := []*observedZjlibTransport{}
	access.create = func(actual time.Time) (zjlib.Transport, func(), error) {
		if actual != deadline {
			t.Fatal("deadline not forwarded")
		}
		created++
		wire := &observedZjlibTransport{FixtureTransport: zjlib.NewFixtureTransport(zjlib.Success)}
		wires = append(wires, wire)
		return wire, func() { closed++ }, nil
	}
	article := domain.ArticleLocator{Title: "Exact Article", JournalTitle: "Fixture CNKI Journal", Authors: []string{"Ada Lovelace", "Grace Hopper"}}
	for range 2 {
		result, err := access.ResolveFullText(context.Background(), article, domain.ArticleAccessContext{UserId: new(identity.Id(9)), Deadline: deadline})
		assertExactZjlibDocument(t, result, err)
	}
	if created != 2 || closed != 2 || wires[0] == wires[1] || wires[0].limit != 10 || wires[1].limit != 10 || before != string(reader.data.SessionData) {
		t.Fatal("session lifecycle or search contract changed")
	}
}

func TestZjlibFullTextSupportAndErrorPriority(t *testing.T) {
	access := NewZjlibFullTextProvider(nil, transport.Proxy{})
	for _, test := range []struct {
		article  domain.ArticleLocator
		expected bool
	}{
		{domain.ArticleLocator{Title: "T", JournalTitle: "J", Authors: []string{" ", " A "}}, true},
		{domain.ArticleLocator{Title: " ", JournalTitle: "J", Authors: []string{"A"}}, false},
		{domain.ArticleLocator{Title: "T", JournalTitle: "J", Authors: []string{" "}}, false},
		{domain.ArticleLocator{Title: "T", Authors: []string{"A"}}, false},
	} {
		if access.SupportsFullText(test.article) != test.expected {
			t.Fatal(test)
		}
	}
	for _, test := range []struct {
		message string
		kind    provider.ErrorKind
	}{{"No exact CNKI full-text match token", provider.NotFound}, {"Run QR login again", provider.AuthenticationRequired}, {"bad token", provider.AuthenticationRequired}, {"Token rejected", provider.TemporarilyUnavailable}, {"upstream failed", provider.TemporarilyUnavailable}} {
		err := mapZjlibProviderError(errors.New(test.message)).(*provider.Error)
		if err.Kind != test.kind || err.Message != "Zhejiang Library CNKI full-text resolution failed" {
			t.Fatal(err)
		}
	}
}

func TestLiveCnkiAccessCreatesAndClosesEachRequest(t *testing.T) {
	access := NewLiveCnkiArticleAccess(nil, transport.Proxy{})
	deadline := time.Now().Add(time.Minute)
	created, closed := 0, 0
	access.create = func(actual time.Time) (cnki.Transport, func(), error) {
		if actual != deadline {
			t.Fatal("deadline not forwarded")
		}
		created++
		return cnki.NewFixtureTransport(cnki.FixtureData{}), func() { closed++ }, nil
	}
	for range 2 {
		_, _ = access.ResolveAbstract(context.Background(), domain.ArticleLocator{Title: "T", JournalTitle: "J"}, domain.ArticleAccessContext{Deadline: deadline})
	}
	if created != 2 || closed != 2 {
		t.Fatal(created, closed)
	}
	access.create = func(time.Time) (cnki.Transport, func(), error) { return nil, nil, errors.New("private proxy URL") }
	_, err := access.ResolveAbstract(context.Background(), domain.ArticleLocator{}, domain.ArticleAccessContext{})
	if !reflect.DeepEqual(err, &provider.Error{Kind: provider.TemporarilyUnavailable, Message: "domestic CNKI transport is unavailable"}) {
		t.Fatal(err)
	}
}

// assertExactZjlibDocument checks the returned kind, MIME type, filename and exact fixture PDF bytes.
func assertExactZjlibDocument(t *testing.T, result domain.ArticleFullTextResolution, err error) {
	t.Helper()
	if err != nil || result.Document == nil || result.Redirect != nil {
		t.Fatalf("%#v %v", result, err)
	}
	if result.Document.ContentType != "application/pdf" || *result.Document.Filename != "Exact Article.pdf" || string(result.Document.Bytes) != "%PDF-1.4\n% fixture cnki pdf\n" {
		t.Fatal(result.Document)
	}
}
