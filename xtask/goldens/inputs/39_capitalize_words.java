class CapitalizeWords {
    String capitalize(String sentence) {
        String[] words = sentence.trim().split(" ");
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < words.length; i++) {
            String word = words[i];
            if (!word.isEmpty()) {
                sb.append(Character.toUpperCase(word.charAt(0))).append(word.substring(1));
            }
            if (i < words.length - 1) {
                sb.append(' ');
            }
        }
        String result = sb.toString();
        System.out.println(result.length());
        return result;
    }
}
