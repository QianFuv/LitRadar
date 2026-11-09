package cnki

import "time"

type requestBudget struct {
	ordinaryAttempts, transportFailures, captchaReplays, attemptsStarted uint64
	hasPendingTransportRetry, hasPendingCaptchaReplay                    bool
}

func (budget *requestBudget) nextAttempt() (bool, bool) {
	if budget.hasPendingCaptchaReplay {
		if budget.captchaReplays >= CaptchaSolveBudget {
			return false, false
		}
		budget.hasPendingCaptchaReplay = false
		budget.captchaReplays++
		budget.attemptsStarted++
		return true, true
	}
	if budget.hasPendingTransportRetry {
		budget.hasPendingTransportRetry = false
		budget.attemptsStarted++
		return true, true
	}
	if budget.ordinaryAttempts >= 5 {
		return false, false
	}
	budget.ordinaryAttempts++
	budget.attemptsStarted++
	return false, true
}
func (budget *requestBudget) scheduleCaptchaReplay() error {
	if budget.captchaReplays >= CaptchaSolveBudget {
		return &Error{Kind: "Request", Message: "domestic CNKI captcha replay budget exhausted"}
	}
	budget.hasPendingCaptchaReplay = true
	return nil
}
func (budget *requestBudget) scheduleTransportRetry() bool {
	budget.transportFailures++
	if budget.transportFailures >= 8 {
		return false
	}
	budget.hasPendingTransportRetry = true
	return true
}
func retryDelay(attempts uint64) time.Duration {
	exponent := uint64(0)
	if attempts > 0 {
		exponent = min(attempts-1, 3)
	}
	return time.Second * time.Duration(uint64(1)<<exponent)
}
func (budget *requestBudget) transportRetryDelay() time.Duration {
	return retryDelay(budget.transportFailures)
}
func (budget *requestBudget) ordinaryRetryDelay() (time.Duration, bool) {
	return retryDelay(budget.ordinaryAttempts), budget.ordinaryAttempts < 5
}
func (budget *requestBudget) didRetry() bool { return budget.attemptsStarted > 1 }
