This folder contains code to generate an STL file of the controller's 3D-printable case.

To generate the STL file, run the following command:

```console
$ uv run main.py
```

To iterate on the model in VS Code:

1. Run the above command at least once to ensure a virtual environment has been created and the required packages installed.
1. In VS Code, select `case/.venv` as the Python interpreter.
1. Ensure the following VS Code extensions are installed:
   - [Jupyter](https://marketplace.visualstudio.com/items?itemName=ms-toolsai.jupyter)
   - [OCP Cad Viewer](https://marketplace.visualstudio.com/items?itemName=bernhard-42.ocp-cad-viewer)
1. Open the OCP CAD Viewer window by clicking on the icon next to `ocp_vscode` in its extension panel.
1. Open `main.py`.
1. Uncomment the `show()` call.
1. Click "Run Cell" at the top of the file (this can take a while the first time).
1. Click the next "Run Cell".
1. Edit the model code and click "Run Cell" (or press ⌃Enter on macOS) to update the model in the viewer.
1. Comment out the `show()` call when you're done.
