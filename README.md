# Tear

The Elm Architecture for Rust.

An implementation of the Elm Architecture for Rust.

## Components

The Elm Architecture consists of a small number of components that play nicely
together. The core element is the model. When the application starts the `init`
function is called to retrieve the initial model. After initialization the
`view` function is called to render the user interface. Once rendering is done
the application will wait for events. Events are emitted by the functions the
application is subscribed to. The `subscriptions` function will emit a number
of such functions based on the state of the model. Each function will run in a
thread of it's own and emit events as relevant.

The model is updated by
the
`update` function as it reacts to messages.
