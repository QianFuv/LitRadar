// Package primitives provides synthetic migration-only listeners and native-helper probes.
package primitives

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"math/big"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"time"

	"github.com/QianFuv/LitRadar/internal/platform/transport"
)

// Run executes an explicitly selected migration fixture; no application commands are simulated.
func Run(mode, directory string) error {
	switch mode {
	case "certificate":
		return certificates(directory)
	case "ledger":
		return ledger(directory)
	case "probe":
		return probe()
	case "helpers":
		return helpers()
	default:
		return errors.New("unknown fixture mode")
	}
}

// certificates creates the directory and publishes the three fixture identities in order.
func certificates(directory string) error {
	if err := os.MkdirAll(directory, 0700); err != nil {
		return err
	}
	for _, identity := range []string{"valid", "wrong-san", "wrong-ca"} {
		if err := writeFixtureCertificateIdentity(directory, identity); err != nil {
			return err
		}
	}
	return nil
}

func ledger(directory string) error {
	var count atomic.Int32
	mux := http.NewServeMux()
	mux.HandleFunc("POST /send", func(writer http.ResponseWriter, request *http.Request) {
		if request.Host != "www.pushplus.plus" || request.TLS == nil || request.TLS.ServerName != "www.pushplus.plus" {
			writer.WriteHeader(400)
			return
		}
		body, err := io.ReadAll(http.MaxBytesReader(writer, request.Body, 4096))
		if err != nil || string(body) != `{"token":"synthetic-only","content":"migration probe"}` {
			writer.WriteHeader(400)
			return
		}
		count.Add(1)
		fmt.Println(`{"event":"synthetic-delivery","host":"www.pushplus.plus","path":"/send"}`)
		writer.Header().Set("Content-Type", "application/json")
		io.WriteString(writer, `{"code":200,"msg":"success"}`)
	})
	mux.HandleFunc("GET /ledger", func(writer http.ResponseWriter, request *http.Request) {
		json.NewEncoder(writer).Encode(map[string]int32{"count": count.Load()})
	})
	identity := os.Getenv("LITRADAR_FIXTURE_CERT")
	if identity == "" {
		identity = "valid"
	}
	if identity != "valid" && identity != "wrong-san" {
		return errors.New("invalid synthetic certificate identity")
	}
	server := &http.Server{Addr: ":443", Handler: mux, ReadHeaderTimeout: 5 * time.Second}
	return server.ListenAndServeTLS(filepath.Join(directory, identity+".pem"), filepath.Join(directory, identity+"-key.pem"))
}

func probe() error {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	connection, err := (&net.Dialer{}).DialContext(ctx, "tcp", "1.1.1.1:443")
	if err == nil {
		connection.Close()
		return errors.New("outbound isolation failed")
	}
	selected, err := transport.New("")
	if err != nil {
		return err
	}
	defer selected.CloseIdleConnections()
	client := &http.Client{Transport: selected, Timeout: 5 * time.Second}
	response, err := client.Post("https://www.pushplus.plus/send", "application/json", strings.NewReader(`{"token":"synthetic-only","content":"migration probe"}`))
	if err != nil {
		return err
	}
	data, err := io.ReadAll(response.Body)
	response.Body.Close()
	if err != nil || response.StatusCode != 200 || string(data) != `{"code":200,"msg":"success"}` {
		return errors.New("synthetic delivery did not reach ledger")
	}
	response, err = client.Get("https://www.pushplus.plus/ledger")
	if err != nil {
		return err
	}
	defer response.Body.Close()
	var result struct{ Count int }
	if err := json.NewDecoder(response.Body).Decode(&result); err != nil {
		return err
	}
	if result.Count != 1 {
		return fmt.Errorf("unexpected ledger count %d", result.Count)
	}
	fmt.Println(`{"fixedHostTls":true,"deliveryCount":1,"outboundDenied":true}`)
	return nil
}

// writeFixtureCertificateIdentity publishes its CA before generating and writing the leaf and key pair.
func writeFixtureCertificateIdentity(directory, identity string) error {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return err
	}
	template := &x509.Certificate{SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: "LitRadar migration fixture"}, NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(24 * time.Hour), IsCA: true, BasicConstraintsValid: true, KeyUsage: x509.KeyUsageCertSign | x509.KeyUsageDigitalSignature}
	root, err := x509.CreateCertificate(rand.Reader, template, template, &key.PublicKey, key)
	if err != nil {
		return err
	}
	if err := os.WriteFile(filepath.Join(directory, identity+"-ca.pem"), pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: root}), 0644); err != nil {
		return err
	}
	leaf := &x509.Certificate{SerialNumber: big.NewInt(2), Subject: pkix.Name{CommonName: "www.pushplus.plus"}, DNSNames: []string{"www.pushplus.plus"}, NotBefore: template.NotBefore, NotAfter: template.NotAfter, KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}}
	if identity == "wrong-san" {
		leaf.DNSNames = []string{"wrong.fixture.invalid"}
	}
	certificate, err := x509.CreateCertificate(rand.Reader, leaf, template, &key.PublicKey, key)
	if err != nil {
		return err
	}
	private, err := x509.MarshalPKCS8PrivateKey(key)
	if err != nil {
		return err
	}
	for name, data := range map[string][]byte{identity + ".pem": pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certificate}), identity + "-key.pem": pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: private})} {
		if err := os.WriteFile(filepath.Join(directory, name), data, 0644); err != nil {
			return err
		}
	}
	return nil
}
