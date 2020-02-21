# Debugging Wok

wok provides a Container Runtime Interface (CRI) for WASM runtimes. This allows users to run native WASM workloads on
Kubernetes.

To test your changes, crictl can be used to mimic the calls a kubelet would make to wok.

NOTE: as wok is still in progress, some of the examples listed here may not work as expected.

## Install crictl

NOTE: crictl is only available on Windows and Linux. There is no current release available for macOS. See
https://github.com/kubernetes-sigs/cri-tools/issues/573 for more details.

crictl can be downloaded from cri-tools [release page](https://github.com/kubernetes-sigs/cri-tools/releases).

It can also be installed using [gofish](https://github.com/fishworks/gofish).

```
$ gofish install crictl
$ crictl --version
crictl version v1.17.0
```

## Using wok with crictl

First, start wok:

```
$ just run
```

If you want to start it with the log level to DEBUG:

```
$ just log_level=wok=debug run
```

In another terminal, start interacting with wok using crictl:

```
$ crictl version
Version:  0.1.0
RuntimeName:  wok
RuntimeVersion:  0.1.0
RuntimeApiVersion:  v1alpha2
```

### Creating a new pod sandbox

```
$ crictl runp contrib/crictl/pod-sandbox-config.json
```

You can also change the runtime handler between WasCC and WASI at runtime, allowing you to test each runtime handler:

```
$ crictl runp contrib/crictl/pod-sandbox-config.json --runtime WASCC
$ crictl runp contrib/crictl/pod-sandbox-config.json --runtime WASI
d736d297-6ec1-4edc-a1b7-acad55cb2806
```

Now let's check the pod sandbox and make sure it is in the "Ready" state:

```
$ crictl pods --no-trunc
POD ID                                 CREATED             STATE               NAME                     NAMESPACE           ATTEMPT
d736d297-6ec1-4edc-a1b7-acad55cb2806   9 minutes ago       Ready               hello-wasm-sandbox       default             1
```


### Inspecting pod sandboxes

```
$ crictl inspectp d736d297-6ec1-4edc-a1b7-acad55cb2806
```

### Pull a WASM image

```
$ crictl pull webassembly.azurecr.io/hello-wasm:v1
Image is up to date for webassembly.azurecr.io/hello-wasm:v1
```

### Create a container in the pod sandbox

```
$ crictl create d736d297-6ec1-4edc-a1b7-acad55cb2806 contrib/crictl/container-config.json contrib/crictl/pod-sandbox-config.json
49479502-f935-4556-ab72-f664a2678edc
```

Now let's check the container and make sure it is in the "Created" state:

```
$ crictl ps -a --no-trunc
CONTAINER                              IMAGE                                  CREATED              STATE               NAME                ATTEMPT             POD ID
49479502-f935-4556-ab72-f664a2678edc   webassembly.azurecr.io/hello-wasm:v1   About a minute ago   Created             hello-wasm          0                   d736d297-6ec1
```

### Start a container

```
$ crictl start 49479502-f935-4556-ab72-f664a2678edc
```

```
$ crictl ps --no-trunc
CONTAINER                              IMAGE                                  CREATED              STATE               NAME                ATTEMPT             POD ID
49479502-f935-4556-ab72-f664a2678edc   webassembly.azurecr.io/hello-wasm:v1   4 minutes ago        Running             hello-wasm          0                   d736d297-6ec1
```

### Shortcut: create and start a container with one command

```
crictl run contrib/crictl/container-config.json contrib/crictl/pod-sandbox-config.json
```

### Cleanup

```
$ crictl stop 49479502-f935-4556-ab72-f664a2678edc
49479502-f935-4556-ab72-f664a2678edc
$ crictl rm 49479502-f935-4556-ab72-f664a2678edc
$ crictl stopp d736d297-6ec1-4edc-a1b7-acad55cb2806
Stopped sandbox d736d297-6ec1-4edc-a1b7-acad55cb2806
$ crictl rmp d736d297-6ec1-4edc-a1b7-acad55cb2806
d736d297-6ec1-4edc-a1b7-acad55cb2806
$ crictl rmi webassembly.azurecr.io/hello-wasm:v1
```

If you want to be absolutely sure you have a clean test environment for the next run:

```
$ rm -rf ~/.wok
```
