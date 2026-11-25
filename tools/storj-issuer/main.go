// Local helper. Only the restricted grant is registered with Storj Edge.
package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"os"
	"strings"
	"time"

	"storj.io/common/encryption"
	"storj.io/common/grant"
	"storj.io/common/macaroon"
	"storj.io/common/paths"
	"storj.io/uplink"
	"storj.io/uplink/edge"
)

type request struct {
	Op, Owner, Child, Bucket, Prefix, Object, Digest string
}

func memberGrant(owner *uplink.Access, bucket, prefix string) (*uplink.Access, error) {
	if bucket == "" || prefix == "" {
		return nil, errors.New("missing scope")
	}
	return owner.Share(uplink.Permission{AllowDownload: true, AllowUpload: true, AllowList: true},
		uplink.SharePrefix{Bucket: bucket, Prefix: prefix})
}

// Evaluate the parent caveats at the directory itself. A grant restricted to a
// single existing object can prove possession of genesis but cannot support sync.
func checkPermissions(ctx context.Context, r request) error {
	access, err := grant.ParseAccess(r.Owner)
	if err != nil {
		return err
	}
	path, err := encryption.EncryptPathWithStoreCipher(r.Bucket, paths.NewUnencrypted(strings.TrimSuffix(r.Prefix, "/")), access.EncAccess.Store)
	if err != nil {
		return err
	}
	for _, op := range []macaroon.ActionType{macaroon.ActionRead, macaroon.ActionList, macaroon.ActionWrite} {
		_, err = access.APIKey.GetAllowedBuckets(ctx, macaroon.Action{Op: op, Time: time.Now(), Bucket: []byte(r.Bucket), EncryptedPath: []byte(path.Raw())})
		if err != nil {
			return err
		}
	}
	return nil
}

func process(ctx context.Context, r request) (any, error) {
	switch r.Op {
	case "prepare":
		owner, err := uplink.ParseAccess(r.Owner)
		if err != nil {
			return nil, err
		}
		child, err := memberGrant(owner, r.Bucket, r.Prefix)
		if err != nil {
			return nil, err
		}
		return child.Serialize()
	case "verify":
		if err := checkPermissions(ctx, r); err != nil {
			return nil, err
		}
		owner, err := uplink.ParseAccess(r.Owner)
		if err != nil {
			return nil, err
		}
		issuer, err := memberGrant(owner, r.Bucket, r.Prefix)
		if err != nil {
			return nil, err
		}
		project, err := uplink.OpenProject(ctx, issuer)
		if err != nil {
			return nil, err
		}
		defer project.Close()
		download, err := project.DownloadObject(ctx, r.Bucket, r.Prefix+r.Object, nil)
		if err != nil {
			return nil, err
		}
		defer download.Close()
		h := sha256.New()
		const maxBytes = 4 * 1024 * 1024
		n, err := io.Copy(h, io.LimitReader(download, maxBytes+1))
		if err != nil || n > maxBytes || hex.EncodeToString(h.Sum(nil)) != r.Digest {
			return nil, errors.New("vault mismatch")
		}
		// Confirm the scoped issuer can revoke descendants on this satellite.
		// This probe has never been registered or delivered to a device.
		probe, err := issuer.Share(uplink.ReadOnlyPermission(), uplink.SharePrefix{Bucket: r.Bucket, Prefix: r.Prefix})
		if err != nil {
			return nil, err
		}
		if err := project.RevokeAccess(ctx, probe); err != nil {
			return nil, err
		}
		return issuer.Serialize()
	case "register":
		child, err := uplink.ParseAccess(r.Child)
		if err != nil {
			return nil, err
		}
		config := edge.Config{AuthServiceAddress: "auth.storjshare.io:7777"}
		credentials, err := config.RegisterAccess(ctx, child, nil)
		if err != nil {
			return nil, err
		}
		return map[string]string{"endpoint": credentials.Endpoint, "access": credentials.AccessKeyID, "secret": credentials.SecretKey}, nil
	case "revoke":
		owner, err := uplink.ParseAccess(r.Owner)
		if err != nil {
			return nil, err
		}
		child, err := uplink.ParseAccess(r.Child)
		if err != nil {
			return nil, err
		}
		p, err := uplink.OpenProject(ctx, owner)
		if err != nil {
			return nil, err
		}
		defer p.Close()
		return nil, p.RevokeAccess(ctx, child)
	default:
		return nil, errors.New("unknown operation")
	}
}

func main() {
	var r request
	raw, err := io.ReadAll(io.LimitReader(os.Stdin, 1024*1024+1))
	if err != nil || len(raw) > 1024*1024 || json.Unmarshal(raw, &r) != nil {
		os.Exit(1)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	response, err := process(ctx, r)
	if err != nil || json.NewEncoder(os.Stdout).Encode(response) != nil {
		os.Exit(1)
	}
}
