class ReverseWords {
    String reverseWords(String sentence) {
        String[] words = sentence.trim().split(" ");
        StringBuilder sb = new StringBuilder();
        for (int i = words.length - 1; i >= 0; i--) {
            sb.append(words[i]);
            if (i > 0) {
                sb.append(' ');
            }
        }
        String result = sb.toString();
        System.out.println(result.length());
        return result;
    }
}
