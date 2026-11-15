# Prints the build sha, the tag publish-image gives the images.
{
  writeShellApplication,
  buildSha,
}:
writeShellApplication {
  name = "build-sha";
  meta.description = "Print this build's sha";
  text = ''
    echo ${buildSha}
  '';
}
