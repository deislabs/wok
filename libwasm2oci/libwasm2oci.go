package main

import (
	"C"

	"github.com/engineerd/wasm-to-oci/pkg/oci"
	log "github.com/sirupsen/logrus"
)

//export Pull
func Pull(ref, outFile string) int64 {
	if err := oci.Pull(ref, outFile); err != nil {
		log.Infof("cannot pull module: %v", err)
		return 1
	}

	return 0
}

func main() {}
