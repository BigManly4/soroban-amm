package gosdk

import (
	"context"
	"errors"
	"strings"
)

// Signer signs transaction envelopes. It is an interface so callers can back it
// with a local keypair, an HSM, or a remote signing service; the client never
// holds a raw secret key.
type Signer interface {
	// Address returns the signer's account address (a G... strkey).
	Address() string
	// SignEnvelope signs a base64-encoded transaction envelope XDR for the
	// given network passphrase and returns the signed envelope, also base64.
	SignEnvelope(ctx context.Context, envelopeXDR string, networkPassphrase string) (string, error)
}

// ErrSignerAddress is returned when a Signer reports an address that is not a
// plausible Stellar account.
var ErrSignerAddress = errors.New("signer address is not a valid account address")

// SignerFunc adapts a function to the Signer interface, pairing it with a fixed
// address.
type SignerFunc struct {
	// Addr is the account address the signing function signs for.
	Addr string
	// Sign performs the signing.
	Sign func(ctx context.Context, envelopeXDR string, networkPassphrase string) (string, error)
}

// Address returns the configured address.
func (s SignerFunc) Address() string { return s.Addr }

// SignEnvelope calls the configured signing function.
func (s SignerFunc) SignEnvelope(ctx context.Context, envelopeXDR string, networkPassphrase string) (string, error) {
	if s.Sign == nil {
		return "", ErrNoSigner
	}
	return s.Sign(ctx, envelopeXDR, networkPassphrase)
}

// ValidateSigner checks that a Signer reports a well-formed account address.
func ValidateSigner(s Signer) error {
	if s == nil {
		return ErrNoSigner
	}
	if !IsAccountAddress(s.Address()) {
		return ErrSignerAddress
	}
	return nil
}

// IsAccountAddress reports whether addr looks like a Stellar account strkey:
// 56 characters beginning with G.
func IsAccountAddress(addr string) bool {
	return len(addr) == 56 && strings.HasPrefix(addr, "G") && isBase32Upper(addr)
}

// IsContractAddress reports whether addr looks like a Soroban contract strkey:
// 56 characters beginning with C.
func IsContractAddress(addr string) bool {
	return len(addr) == 56 && strings.HasPrefix(addr, "C") && isBase32Upper(addr)
}

func isBase32Upper(s string) bool {
	for _, r := range s {
		switch {
		case r >= 'A' && r <= 'Z':
		case r >= '2' && r <= '7':
		default:
			return false
		}
	}
	return true
}
