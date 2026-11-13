# AI images

desigma can generate images and edit parts of images with a model you choose on
[OpenRouter](https://openrouter.ai). It is off until you add your own API key; desigma has no
account, no AI service and no usage of its own.

## Set up

1. Create an API key at [openrouter.ai/keys](https://openrouter.ai/keys) and add credits.
2. In desigma open the main menu → **Preferences → AI images…**.
3. Paste the key, press **Load models** and pick a model. Models that accept reference images
   are listed; those are the ones that can edit a picture.
4. **Save**.

![The AI images settings](images/desigma-ai-settings.png)

The key is kept in the operating system's credential store: Windows Credential Manager, the
macOS Keychain, or the Secret Service (GNOME Keyring, KWallet) on Linux. It is never written to
`settings.json`, a document, a log or a collaboration session. **Remove key** deletes it.

On a machine without a credential store, set the `OPENROUTER_API_KEY` environment variable
before starting desigma; it takes precedence over a stored key.

## Generate an image

**Object → Generate image…**, or right-click the canvas.

- Type a prompt and choose a ratio (**Auto** leaves it to the model). `Ctrl/Cmd+Enter` sends it.
- With layers selected, up to four of them are rendered and sent as reference images.
- With one empty rectangle, ellipse or frame selected, the result becomes its image fill and
  the ratio closest to the shape is preselected.
- Otherwise the image is placed in the middle of the view.

## Edit part of an image

Select one layer with an image fill, then **Object → Edit image with AI…**.

- Drag over the preview to mark the part to change; click the preview to clear the mark.
- Say what should change and press **Generate**.

desigma sends the whole picture with your request and the marked area. From the answer it takes
only the marked rectangle, blended at its edge, and pastes it into the original at the
original's resolution. Pixels outside the rectangle are never changed. How well the new content
matches its surroundings depends on the model. Without a mark the whole picture is replaced by
the model's version.

Each generation is one Undo step. The original picture stays in the file, so Undo brings it back.

## What is sent

To OpenRouter (and through it to the model's provider), only when you press **Generate**:

- your prompt,
- the reference images: the selected layers as PNG, or the picture being edited (scaled down to
  2048 px on its longest side),
- your API key, as the request's authorization.

**Load models** asks OpenRouter for its list of image models with the same key. Nothing else
leaves your computer, and the cost of each image is charged to your OpenRouter account.

## Limits

- One image per request.
- A request can take a minute or two; **Cancel** closes the dialog and ignores the answer (the
  provider may still charge for a generation that had already started).
- Viewers in a collaboration session cannot generate; everyone else's result reaches the other
  participants like any other image.
