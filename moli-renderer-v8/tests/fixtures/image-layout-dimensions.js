(() => Object.fromEntries([...document.images].map(image => [
  image.id, [image.width, image.height, image.naturalWidth, image.naturalHeight]
])))()
