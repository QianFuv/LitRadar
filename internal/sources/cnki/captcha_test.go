package cnki

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/sources/jfbym"
)

func testPuzzle() CaptchaPuzzle {
	return CaptchaPuzzle{ChallengeUrl: KnsBase + "/verify/home", CaptchaType: "blockPuzzle", CaptchaId: "secret-id", Ident: "private-ident", ReturnUrl: "private-return", SecretKey: "0123456789abcdef", Token: "private-token", OriginalImageBase64: "original", JigsawImageBase64: "jigsaw"}
}

func TestCaptchaFreshPuzzlePerCandidate(t *testing.T) {
	session := NewCaptchaSession(5)
	solver := jfbym.NewFixture(100.4, 0)
	fetches := 0
	submits := []string{}
	fetch := func(context.Context, string) (CaptchaPuzzle, error) {
		fetches++
		puzzle := testPuzzle()
		puzzle.CaptchaId = fmt.Sprintf("id-%d", fetches)
		return puzzle, nil
	}
	submit := func(_ context.Context, puzzle CaptchaPuzzle, point string) (bool, error) {
		submits = append(submits, point)
		return len(submits) == 3, nil
	}
	if err := session.SolveChallenge(context.Background(), KnsBase+"/verify/home", solver, fetch, submit); err != nil {
		t.Fatal(err)
	}
	if fetches != 3 || session.RemainingBudget() != 2 {
		t.Fatalf("fetches=%d remaining=%d", fetches, session.RemainingBudget())
	}
	for index, x := range []int32{100, 101, 99} {
		want, err := jfbym.EncryptPointJson(testPuzzle().SecretKey, x, 5)
		if err != nil {
			t.Fatal(err)
		}
		if submits[index] != want {
			t.Fatalf("candidate %d ciphertext differs", index)
		}
	}
	for input, want := range map[string]string{KnsBase + "/a?x=%2b+~": KnsBase + "/a?x=%2b+~&captchaId=id-3", KnsBase + "/a?%63aptchaId=old": KnsBase + "/a?%63aptchaId=old", KnsBase + "/a?CAPTCHAID=old": KnsBase + "/a?CAPTCHAID=old&captchaId=id-3", KnsBase + "/a?x=1&": KnsBase + "/a?x=1&&captchaId=id-3"} {
		got, err := session.AttachCaptchaId(input)
		if err != nil || got != want {
			t.Fatalf("attach %q: %q %v want %q", input, got, err, want)
		}
	}
	clone := session.Clone()
	if err := session.SolveChallenge(context.Background(), "url", solver, fetch, func(context.Context, CaptchaPuzzle, string) (bool, error) { return false, nil }); err == nil || !strings.Contains(err.Error(), "after 5 attempts") {
		t.Fatal(err)
	}
	if clone.RemainingBudget() != 2 || session.RemainingBudget() != 0 {
		t.Fatal("clone shared budget")
	}
	retained, _ := session.AttachCaptchaId(KnsBase + "/a")
	if !strings.HasSuffix(retained, "captchaId=id-3") {
		t.Fatal(retained)
	}
}

func TestCaptchaFailuresConsumeOneAttempt(t *testing.T) {
	for _, stage := range []string{"fetch", "validate", "solver", "encrypt", "submit"} {
		t.Run(stage, func(t *testing.T) {
			session := NewCaptchaSession(0)
			solver := jfbym.NewFixture(1, 0)
			if stage == "solver" {
				solver = jfbym.NewFixture(1, 1)
			}
			calls := 0
			err := session.SolveChallenge(context.Background(), "url", solver, func(context.Context, string) (CaptchaPuzzle, error) {
				calls++
				puzzle := testPuzzle()
				if stage == "fetch" {
					return puzzle, &Error{Kind: "Request", Message: "fetch"}
				}
				if stage == "validate" {
					puzzle.Token = ""
				}
				if stage == "encrypt" {
					puzzle.SecretKey = "汉字汉字abcd"
				}
				return puzzle, nil
			}, func(context.Context, CaptchaPuzzle, string) (bool, error) {
				return false, &Error{Kind: "Request", Message: "submit"}
			})
			if err == nil || calls != 1 || session.RemainingBudget() != 0 {
				t.Fatalf("%v calls=%d remaining=%d", err, calls, session.RemainingBudget())
			}
		})
	}
}

func TestCaptchaFormattingAndCancelledSolve(t *testing.T) {
	puzzle := testPuzzle()
	for _, value := range []any{puzzle, &puzzle} {
		for _, format := range []string{"%v", "%+v", "%#v"} {
			text := fmt.Sprintf(format, value)
			for _, secret := range []string{puzzle.CaptchaId, puzzle.SecretKey, puzzle.Token, puzzle.Ident, puzzle.ReturnUrl} {
				if strings.Contains(text, secret) {
					t.Fatal(text)
				}
			}
		}
	}
	session := NewCaptchaSession(5)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := session.SolveChallenge(ctx, "url", nil, nil, nil); err == nil || session.RemainingBudget() != 5 {
		t.Fatal(err)
	}
	var workers sync.WaitGroup
	for range 4 {
		workers.Go(func() {
			for range 30 {
				_ = fmt.Sprintf("%#v", session)
				_ = session.RemainingBudget()
			}
		})
	}
	workers.Wait()
}

func TestRequestBudgetSeparatesRetryClasses(t *testing.T) {
	var budget requestBudget
	for ordinary := 0; ordinary < 5; ordinary++ {
		isReplay, ok := budget.nextAttempt()
		if !ok || isReplay {
			t.Fatal("ordinary attempt missing")
		}
		if ordinary == 0 {
			if !budget.scheduleTransportRetry() {
				t.Fatal("transport retry missing")
			}
			if err := budget.scheduleCaptchaReplay(); err != nil {
				t.Fatal(err)
			}
			for range 2 {
				replay, ok := budget.nextAttempt()
				if !replay || !ok {
					t.Fatal("replay missing")
				}
			}
		}
	}
	if _, ok := budget.nextAttempt(); ok {
		t.Fatal("ordinary budget exceeded")
	}
	if _, ok := budget.ordinaryRetryDelay(); ok {
		t.Fatal("delay after exhaustion")
	}
	for range 4 {
		if err := budget.scheduleCaptchaReplay(); err != nil {
			t.Fatal(err)
		}
		if replay, ok := budget.nextAttempt(); !replay || !ok {
			t.Fatal("captcha replay used ordinary budget")
		}
	}
	var failure *Error
	if !errors.As(budget.scheduleCaptchaReplay(), &failure) {
		t.Fatal("captcha replay budget exceeded")
	}
	for range 6 {
		if !budget.scheduleTransportRetry() {
			t.Fatal("transport stopped early")
		}
		budget.nextAttempt()
	}
	if budget.scheduleTransportRetry() {
		t.Fatal("transport budget exceeded")
	}
	if !budget.didRetry() {
		t.Fatal("retry marker lost")
	}
}

func TestCaptchaCallbacksCanInspectSession(t *testing.T) {
	session := NewCaptchaSession(5)
	done := make(chan error, 1)
	go func() {
		done <- session.SolveChallenge(context.Background(), "url", jfbym.NewFixture(1, 0), func(context.Context, string) (CaptchaPuzzle, error) {
			_ = session.RemainingBudget()
			_ = fmt.Sprintf("%#v", session)
			return testPuzzle(), nil
		}, func(context.Context, CaptchaPuzzle, string) (bool, error) {
			_, err := session.AttachCaptchaId(KnsBase + "/a")
			return true, err
		})
	}()
	select {
	case err := <-done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(time.Second):
		t.Fatal("captcha callback cannot inspect its session")
	}
}

func TestCaptchaQueryIsNotChallengePath(t *testing.T) {
	if LooksLikeCaptchaChallenge("x", "?/verify/home") {
		t.Fatal("empty URL path became query text")
	}
}
