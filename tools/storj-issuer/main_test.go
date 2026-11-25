package main

import (
	"bytes"
	"context"
	"testing"
	"time"

	"storj.io/common/grant"
	"storj.io/common/macaroon"
	"storj.io/common/storj"
)

func TestDeviceGrantScope(t *testing.T) {
	secret, err := macaroon.NewSecret()
	if err != nil {
		t.Fatal(err)
	}
	api, err := macaroon.NewAPIKey(secret)
	if err != nil {
		t.Fatal(err)
	}
	encryption := grant.NewEncryptionAccessWithDefaultKey(&storj.Key{})
	encryption.SetDefaultPathCipher(storj.EncNull)
	owner := grant.Access{SatelliteAddress: storj.NodeID{1}.String() + "@satellite.example:7777", APIKey: api, EncAccess: encryption}
	raw, err := owner.Serialize()
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	req := request{Op: "prepare", Owner: raw, Bucket: "vault", Prefix: "spaces/test/objects/"}
	if err := checkPermissions(ctx, req); err != nil {
		t.Fatal(err)
	}
	readOnly, err := owner.Restrict(grant.Permission{AllowDownload: true, AllowList: true})
	if err != nil {
		t.Fatal(err)
	}
	readOnlyRaw, err := readOnly.Serialize()
	if err != nil {
		t.Fatal(err)
	}
	badRequest := req
	badRequest.Owner = readOnlyRaw
	if checkPermissions(ctx, badRequest) == nil {
		t.Fatal("read-only parent accepted for sync")
	}
	issuer, err := process(ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	req.Owner = issuer.(string)
	scoped, err := grant.ParseAccess(req.Owner)
	if err != nil {
		t.Fatal(err)
	}
	first, err := process(ctx, req)
	if err != nil {
		t.Fatal(err)
	}
	second, err := process(ctx, req)
	if err != nil || second == first {
		t.Fatal("devices must have distinct grants")
	}
	device, err := grant.ParseAccess(first.(string))
	if err != nil {
		t.Fatal(err)
	}
	for _, check := range []struct {
		op           macaroon.ActionType
		bucket, path string
		allowed      bool
	}{
		{macaroon.ActionRead, "vault", "spaces/test/objects/a", true},
		{macaroon.ActionWrite, "vault", "spaces/test/objects/a", true},
		{macaroon.ActionList, "vault", "spaces/test/objects/", true},
		{macaroon.ActionDelete, "vault", "spaces/test/objects/a", false},
		{macaroon.ActionRead, "vault", "spaces/other/objects/a", false},
		{macaroon.ActionWrite, "vault", "spaces/test/pairing/a", false},
		{macaroon.ActionRead, "other", "spaces/test/objects/a", false},
	} {
		action := macaroon.Action{Op: check.op, Time: time.Now(), Bucket: []byte(check.bucket), EncryptedPath: []byte(check.path)}
		err = device.APIKey.Check(ctx, secret, macaroon.APIKeyVersionObjectLock, action, nil)
		if (err == nil) != check.allowed {
			t.Errorf("unexpected permission for %v", check)
		}
	}

	sibling, err := grant.ParseAccess(second.(string))
	if err != nil {
		t.Fatal(err)
	}
	for _, child := range []*grant.Access{device, sibling} {
		m, err := macaroon.ParseMacaroon(child.APIKey.SerializeRaw())
		if err != nil {
			t.Fatal(err)
		}
		tails := m.Tails(secret)
		if len(tails) < 2 || !bytes.Equal(tails[len(tails)-2], scoped.APIKey.Tail()) {
			t.Fatal("scoped issuer is not the child's immediate revocation authority")
		}
	}
	revoked := revokedTail(device.APIKey.Tail())
	action := macaroon.Action{Op: macaroon.ActionRead, Time: time.Now(), Bucket: []byte("vault"), EncryptedPath: []byte("spaces/test/objects/a")}
	if device.APIKey.Check(ctx, secret, macaroon.APIKeyVersionObjectLock, action, revoked) == nil {
		t.Fatal("revoked device retained access")
	}
	if err := sibling.APIKey.Check(ctx, secret, macaroon.APIKeyVersionObjectLock, action, revoked); err != nil {
		t.Fatal("revoking one device denied its sibling")
	}
}

type revokedTail []byte

func (revoked revokedTail) Check(_ context.Context, tails [][]byte) (bool, error) {
	for _, tail := range tails {
		if bytes.Equal(revoked, tail) {
			return true, nil
		}
	}
	return false, nil
}
