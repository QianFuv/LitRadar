package cnki

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"io"
	"net"
	"net/url"
)

// transportFailureKind identifies actionable failures without retaining URLs or certificate details.
func transportFailureKind(failure error) string {
	var request *url.Error
	if errors.As(failure, &request) {
		return transportFailureKind(request.Err)
	}
	var hostname x509.HostnameError
	var validity x509.CertificateInvalidError
	var authority x509.UnknownAuthorityError
	var certificate *tls.CertificateVerificationError
	var network net.Error
	var dns *net.DNSError
	switch {
	case errors.Is(failure, context.Canceled):
		return "cancelled"
	case errors.As(failure, &hostname):
		return "tls_hostname"
	case errors.As(failure, &validity) && validity.Reason == x509.Expired:
		return "tls_validity"
	case errors.As(failure, &authority):
		return "tls_unknown_authority"
	case errors.As(failure, &certificate):
		return "tls_certificate"
	case errors.As(failure, &network) && network.Timeout():
		return "timeout"
	case errors.As(failure, &dns):
		return "dns"
	case errors.Is(failure, io.EOF), errors.Is(failure, io.ErrUnexpectedEOF):
		return "eof"
	default:
		return "transport"
	}
}
